// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

package cube

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestSandboxSnapshotResponseAlwaysEmitsPath(t *testing.T) {
	// Strict deserialisers (CubeAPI, swarm) error on a missing `path`
	// field; the response struct must always serialise it, even when
	// the caller-derived path string is empty.
	rsp := &sandboxSnapshotResponse{
		SandboxID:  "sb-1",
		SnapshotID: "ckpt-x",
	}
	out, err := json.Marshal(rsp)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if !strings.Contains(string(out), `"path":""`) {
		t.Fatalf("path field must always be serialised; got %s", out)
	}
}

func TestCubeSnapshotRootDefault(t *testing.T) {
	t.Setenv("CUBE_SNAPSHOT_ROOT", "")
	if got := cubeSnapshotRoot(); got != defaultCubeSnapshotRoot {
		t.Fatalf("default snapshot root = %q, want %q", got, defaultCubeSnapshotRoot)
	}
}

func TestCubeSnapshotRootEnvOverride(t *testing.T) {
	t.Setenv("CUBE_SNAPSHOT_ROOT", "/tmp/alt-cube-snapshot/cubebox")
	if got := cubeSnapshotRoot(); got != "/tmp/alt-cube-snapshot/cubebox" {
		t.Fatalf("env override snapshot root = %q", got)
	}
}

func TestCubeSnapshotRootTrimsWhitespace(t *testing.T) {
	t.Setenv("CUBE_SNAPSHOT_ROOT", "   ")
	if got := cubeSnapshotRoot(); got != defaultCubeSnapshotRoot {
		t.Fatalf("whitespace-only override should fall back to default, got %q", got)
	}
}
