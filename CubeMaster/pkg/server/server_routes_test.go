// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

package server

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gorilla/mux"

	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/service/httpservice/cube"
)

// TestSnapshotRoutesAreRegistered guards the bug from
// 782e721 ("fix: register snapshot routes in CubeMaster mux"): the
// cube.HttpHandler switch handled the snapshot paths, but the gorilla
// mux subrouter never had matching HandleFunc entries — so every
// request returned the http package's default `404 page not found`
// before the handler ever ran.  Symptom downstream: dyson-swarm's
// in-place rotation pipeline hung in the snapshot phase, surfacing as
// a 502 to the SPA after the proxy gave up.
//
// Each subtest builds the production router via registerCubeRoutes
// (the same function registerHandlers calls) and asserts that
// matchedRouteFor returns a non-empty path template — i.e. mux has a
// route for that exact path/method.  We deliberately don't dispatch
// the request, so no DB / cubelet / redis state is needed.
func TestSnapshotRoutesAreRegistered(t *testing.T) {
	r := mux.NewRouter()
	registerCubeRoutes(r)

	cases := []struct {
		name   string
		method string
		path   string
	}{
		{"snapshot create", http.MethodPost, "/cube" + cube.SandboxSnapshotAction},
		{"snapshot delete", http.MethodPost, "/cube" + cube.SandboxSnapshotDeleteAction},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			req := httptest.NewRequest(tc.method, tc.path, nil)
			var match mux.RouteMatch
			if !r.Match(req, &match) {
				t.Fatalf("no route matched %s %s — gorilla mux will 404 the request before cube.HttpHandler runs", tc.method, tc.path)
			}
			if match.Route == nil {
				t.Fatalf("matched but no route attached for %s %s", tc.method, tc.path)
			}
			tmpl, _ := match.Route.GetPathTemplate()
			if tmpl == "" {
				t.Fatalf("matched a route with no path template for %s %s", tc.method, tc.path)
			}
		})
	}
}

// TestSnapshotRoutesNotShadowedByEarlierMatch double-checks that the
// snapshot routes resolve to the cube subrouter rather than to one of
// the broader prefixes registered earlier (e.g. /cube/sandbox).  The
// historical fix added the HandleFunc lines after SandboxCommitAction;
// if a future edit reorders them above SandboxAction's catch-all-ish
// methods, the request might match a sibling instead of the snapshot
// handler.  We assert the resolved route's path template equals the
// snapshot path exactly.
func TestSnapshotRoutesNotShadowedByEarlierMatch(t *testing.T) {
	r := mux.NewRouter()
	registerCubeRoutes(r)

	cases := []struct {
		method string
		path   string
	}{
		{http.MethodPost, "/cube" + cube.SandboxSnapshotAction},
		{http.MethodPost, "/cube" + cube.SandboxSnapshotDeleteAction},
	}
	for _, tc := range cases {
		req := httptest.NewRequest(tc.method, tc.path, nil)
		var match mux.RouteMatch
		if !r.Match(req, &match) {
			t.Fatalf("no route for %s %s", tc.method, tc.path)
		}
		got, _ := match.Route.GetPathTemplate()
		if got != tc.path {
			t.Errorf("%s %s resolved to %q, want %q (a sibling route is shadowing the snapshot handler)", tc.method, tc.path, got, tc.path)
		}
	}
}
