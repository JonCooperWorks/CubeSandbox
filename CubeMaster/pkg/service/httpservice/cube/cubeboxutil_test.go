// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

package cube

import (
	"context"
	"testing"

	cubebox "github.com/tencentcloud/CubeSandbox/CubeMaster/api/services/cubebox/v1"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/base/constants"
	"github.com/tencentcloud/CubeSandbox/CubeMaster/pkg/service/sandbox/types"
)

var _ = cubebox.InstanceType_cubebox

// Gap 2: handleColdStartCompatibility must synthesise a stable netID
// when neither the inbound request nor the host CubeBoxReqTemplate
// carries one — instead of erroring out the way it used to.
func TestColdStartSynthesisesNetIDWhenTemplateMissing(t *testing.T) {
	req := &types.CreateCubeSandboxReq{
		Annotations: map[string]string{},
	}
	if err := handleColdStartCompatibility(req); err != nil {
		t.Fatalf("handleColdStartCompatibility returned error: %v", err)
	}
	got := req.Annotations[constants.AnnotationsNetID]
	if got == "" {
		t.Fatalf("netID was not synthesised; annotations=%v", req.Annotations)
	}
	if got != defaultColdStartNetID {
		t.Fatalf("netID = %q, want %q", got, defaultColdStartNetID)
	}
}

func TestColdStartLeavesInboundNetIDUntouched(t *testing.T) {
	req := &types.CreateCubeSandboxReq{
		Annotations: map[string]string{
			constants.AnnotationsNetID: "caller-supplied",
		},
	}
	if err := handleColdStartCompatibility(req); err != nil {
		t.Fatalf("handleColdStartCompatibility error: %v", err)
	}
	if got := req.Annotations[constants.AnnotationsNetID]; got != "caller-supplied" {
		t.Fatalf("netID = %q, want caller-supplied", got)
	}
}

func TestColdStartHandlesNilAnnotations(t *testing.T) {
	req := &types.CreateCubeSandboxReq{}
	if err := handleColdStartCompatibility(req); err != nil {
		t.Fatalf("handleColdStartCompatibility error: %v", err)
	}
	if req.Annotations == nil {
		t.Fatalf("annotations map should be initialised")
	}
	if req.Annotations[constants.AnnotationsNetID] != defaultColdStartNetID {
		t.Fatalf("netID not set on nil-annotation request")
	}
}

// Gap 3 helper: preserve+restore round-trips both shim annotations
// even when an intervening pass (templatecenter merge) would have
// rewritten them.
func TestPreserveAndRestoreShimAnnotationsRoundTrip(t *testing.T) {
	in := map[string]string{
		shimAnnoAppSnapshotRestore: "true",
		shimAnnoVMSnapshotBasePath: "/var/snap/x",
		"unrelated":                "drop-me",
	}
	preserved := preserveShimRestoreAnnotations(in)
	if preserved[shimAnnoAppSnapshotRestore] != "true" {
		t.Fatalf("restore annotation not preserved")
	}
	if preserved[shimAnnoVMSnapshotBasePath] != "/var/snap/x" {
		t.Fatalf("base.path annotation not preserved")
	}
	if _, ok := preserved["unrelated"]; ok {
		t.Fatalf("unrelated annotations should not be captured")
	}

	// Simulate a template merge that overwrote the shim annotations.
	req := &types.CreateCubeSandboxReq{
		Annotations: map[string]string{
			shimAnnoAppSnapshotRestore: "false-from-template",
			shimAnnoVMSnapshotBasePath: "/wrong/path",
		},
	}
	restoreShimRestoreAnnotations(req, preserved)
	if req.Annotations[shimAnnoAppSnapshotRestore] != "true" {
		t.Fatalf("restore annotation not re-asserted")
	}
	if req.Annotations[shimAnnoVMSnapshotBasePath] != "/var/snap/x" {
		t.Fatalf("base.path annotation not re-asserted")
	}
}

// Gap 3 (negative): if the restore request lacks a templateID
// annotation we still want to skip template hydration and fall through
// to cold-start compat (so swarm-driven shim-only restores keep
// working with the on-disk bundle path alone).
func TestSnapshotRestoreWithoutTemplateIDStillSetsNetID(t *testing.T) {
	origGetTemplateRequestFn := getTemplateRequestFn
	t.Cleanup(func() {
		getTemplateRequestFn = origGetTemplateRequestFn
	})
	getTemplateRequestFn = func(_ context.Context, _ string) (*types.CreateCubeSandboxReq, error) {
		t.Fatalf("getTemplateRequestFn should not be invoked when templateID is absent")
		return nil, nil
	}

	req := &types.CreateCubeSandboxReq{
		InstanceType: cubebox.InstanceType_cubebox.String(),
		Annotations: map[string]string{
			shimAnnoAppSnapshotRestore: "true",
			shimAnnoVMSnapshotBasePath: "/var/snap/y",
		},
	}
	if err := dealCubeboxCreateReqWithTemplate(context.Background(), req); err != nil {
		t.Fatalf("dealCubeboxCreateReqWithTemplate: %v", err)
	}
	if req.Annotations[constants.AnnotationsNetID] == "" {
		t.Fatalf("netID not populated after templateless restore")
	}
	if req.Annotations[shimAnnoVMSnapshotBasePath] != "/var/snap/y" {
		t.Fatalf("shim base.path annotation lost on templateless restore")
	}
}

func TestPreserveShimRestoreAnnotationsHandlesNilAndAbsent(t *testing.T) {
	if got := preserveShimRestoreAnnotations(nil); got != nil {
		t.Fatalf("preserveShimRestoreAnnotations(nil) = %v, want nil", got)
	}
	got := preserveShimRestoreAnnotations(map[string]string{"unrelated": "v"})
	if len(got) != 0 {
		t.Fatalf("preserveShimRestoreAnnotations should not capture unrelated keys: %v", got)
	}
}
