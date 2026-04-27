# Snapshot MVP — fork plan

Fork-local planning doc. Not for upstream merge. Lives under `fork-notes/`
to keep it out of the upstream `docs/` publish path.

## TL;DR

Almost everything already exists. The work is **exposing a small new HTTP
surface in CubeAPI**, plus **one new restore-by-path seam** in CubeMaster.
No new persistent state in Cube. The orchestrator owns snapshot identity.

## Goal

End-to-end live-VM snapshot + restore through CubeAPI, on a single host,
with crash-consistent semantics. Just enough for an external orchestrator
to do:

```
POST   /sandboxes                          { templateID }            -> sandboxID
POST   /sandboxes/:sandboxID/snapshots     { snapshotID }            -> { path }
DELETE /sandboxes/:sandboxID
POST   /sandboxes                          { fromSnapshot: { path } } -> sandboxID'
DELETE /sandboxes/snapshots                { path }                  -> 204
```

Restored sandbox resumes from the captured memory + FS state. Same host
only. The `snapshotID` is the orchestrator's identifier; Cube doesn't
track it. Cube returns the on-disk path and treats it as opaque caller
context on the way back in.

## Scope cuts

**In:** live-VM snapshot of a running sandbox; restore from snapshot
into a new sandbox on the same host; crash-consistent capture.

**Out (deferred):** cross-host transfer / shared blob store; in-guest
quiesce hooks (`PreSnapshot`/`PostSnapshot`); application consistency;
multi-tenancy beyond existing CubeAPI key check; arm64.

## Verified facts (what's already there)

All confirmed by reading the code:

- **Hypervisor primitives** (`hypervisor/vmm/src/api/mod.rs`): `VmSnapshot`
  (line 386), `VmRestore` (line 389), `VmPauseToSnapshot` (line 320),
  `VmResumeFromSnapshot`, `VmSnapshotConfig` (line 244), `RestoreConfig`.
  HTTP-exposed via `hypervisor/vmm/src/api/http/http_endpoint.rs`. Format
  versioned at `SNAPSHOT_VERSION = "1.0.3"` (line 54). Bundle is a
  directory: `config.json`, `memory-ranges`, `state.json` (per
  `hypervisor/docs/snapshot_restore.md`).

- **Shim live-VM snapshot** (`CubeShim/shim/src/snapshot/mod.rs:116`):
  `Snapshot::do_app_snapshot()` does `pause → snapshot → resume` against
  a running VM by sandbox id, talking to the hypervisor's HTTP API over
  a Unix socket. `Sandbox::create_snapshot` at `sb.rs:1136` wraps the
  same primitive directly.

- **Shim restore into a fresh hypervisor instance** is the **load-bearing
  primitive of Cube**. Every fast boot from a template is a restore.
  `Sandbox::restore_vm` at `sb.rs:804`; wire call in `cube_hypervisor.rs:167`
  (`send_request(ApiRequest::VmRestore(restore_config))`). Production-tested.

- **Cubelet `CommitSandbox` RPC** (`Cubelet/services/cubebox/template_ops.go:27`):
  fully implemented — takes `(SandboxID, TemplateID, SnapshotDir)`,
  returns `(SnapshotPath)`. Calls `executeCubeRuntimeSnapshot` which
  invokes `cube-runtime snapshot --app-snapshot --vm-id <id> --path <dest>`.
  Already used by `CubeMaster/pkg/templatecenter/template_commit.go:146`
  for the template-commit feature.

- **Pause/resume in the existing public API actually round-trip a disk
  snapshot.** `Sandbox::pause_vm` at `sb.rs:1153` calls `pause_vm_cube`
  (`cube_hypervisor.rs:270`) which sends `VmPauseToSnapshot`. The VM is
  fully serialized to disk and the hypervisor torn down. Resume restores.
  This **frees RAM** — earlier audit's "CPU-paused, memory-resident"
  claim was wrong.

- **CubeAPI handler scaffold** (`CubeAPI/src/handlers/sandboxes.rs:662`):
  `create_snapshot` route is wired and reaches CubeMaster. Falls back to
  a placeholder UUID at lines 734–752 when CubeMaster says "endpoint
  missing." Clients today get a UUID with no snapshot behind it. Killing
  this fallback is part of the MVP.

## What's actually missing

Three things, none of them require new persistent state, new protos,
or any Cubelet changes:

1. **CubeMaster HTTP endpoint that exposes `CommitSandbox`.** The
   internal call wiring already exists (`pkg/cubelet/actions.go:59`,
   used by templatecenter). The CubeAPI placeholder is hardcoded to
   call `POST /cube/sandbox/snapshot` — match that path.

2. **CubeMaster HTTP endpoint that creates a sandbox from a
   caller-supplied snapshot path.** The shim already supports this
   via two annotations (verified):
   - `cube.vm.snapshot.base.path` — sets the restore source directory
     (`CubeShim/shim/src/sandbox/config.rs:27`, `:113`).
   - `cube.appsnapshot.restore` — forces restore-or-fail mode
     (`CubeShim/shim/src/sandbox/config.rs:29`, `sb.rs:759–767`).
   `RunCubeSandboxRequest.annotations` flows verbatim from CubeMaster
   through Cubelet to the shim (`Cubelet/services/cubebox/cube_container_create.go:156`).
   Cubelet declares the keys as first-class constants
   (`Cubelet/pkg/constants/const.go:226–231`). Pure orchestration: the
   new handler injects these annotations and calls existing `Run`.

3. **CubeAPI plumbing.** Kill the placeholder, expose snapshot create
   with caller-provided ID, expose create-from-snapshot, expose
   delete-snapshot.

## Architecture

```
Orchestrator                    Cube fork
─────────────                   ─────────
DB:                             CubeAPI (HTTP)
  snapshots                       │
    snapshot_id ─► path           ▼
                                CubeMaster (gRPC)
                                  │
                                  ▼
                                Cubelet (gRPC)
                                  │
                                  ▼
                                cube-runtime / shim
                                  │
                                  ▼
                                hypervisor (KVM)

Snapshot blobs land at:
  /var/lib/cube/snapshots/<orchestrator-supplied-id>/
```

Cube is stateless about snapshot identity. The orchestrator's `snapshot_id`
is passed in as the `TemplateID` field on `CommitSandbox`, which controls
the on-disk directory naming. Cube never tracks the relationship.

## Storage model — A3 (orchestrator owns the index)

- **On-node path:** `/var/lib/cube/snapshots/<orchestrator-id>/<resource-spec>/`
  — Cloud Hypervisor's native bundle. Layout follows what `CommitSandbox`
  already produces for templates; we just point it at a snapshots
  directory instead of the templates directory.
- **No Cube-side index.** No new tables in CubeMaster. No new entries
  in any template registry. Snapshots are pure filesystem objects from
  Cube's perspective.
- **Orchestrator's index** (lives in the orchestrator's own DB, separate
  repo): `snapshot_id → (path, source_sandbox_id, created_at)`. Manages
  TTL, pinning, lineage if/when those features land. None of that is
  Cube's problem.
- **Lifecycle:** orchestrator deletes via `DELETE /sandboxes/snapshots {path}`,
  Cube `os.RemoveAll`s the directory. No GC.

## Work breakdown

Three PR-shaped commit series. Each rebases independently against
upstream `main`, each individually proposable.

### Chunk A — CubeMaster HTTP exposure

Files: `CubeMaster/pkg/service/httpservice/cube/` (new
`sandbox_snapshot.go`), plus action constants + dispatch case in
`cube.go`.

- `POST /cube/sandbox/snapshot` — accepts `(sandbox_id, snapshot_id)`,
  forwards to existing internal `cubelet.CommitSandbox` action with
  `TemplateID = snapshot_id` and `SnapshotDir = <snapshots base>`.
  Returns `{ path }`.
- `POST /cube/sandbox/from-snapshot` — accepts `(snapshot_path,
  sandbox_spec)`, builds a normal `RunCubeSandboxRequest` with two
  annotations injected:
  ```
  cube.vm.snapshot.base.path = <snapshot_path>
  cube.appsnapshot.restore   = "true"
  ```
  and calls the existing internal Run path. No proto changes, no
  Cubelet code changes.
- `DELETE /cube/sandbox/snapshot` — accepts `(path)`, validates
  containment under the snapshots base dir, `RemoveAll`s.
- Mirror auth scoping from existing sandbox endpoints.

### Chunk B — CubeAPI: kill the placeholder, expose restore + delete

Files: `CubeAPI/src/handlers/sandboxes.rs`, `CubeAPI/src/routes.rs`,
`CubeAPI/src/cubemaster/`.

- Remove the `is_endpoint_missing()` fallback at lines 734–752. Return
  503/501 when CubeMaster genuinely lacks the endpoint.
- Add optional `fromSnapshot.path` to the create-sandbox request. When
  present, route to the new CubeMaster `POST /cube/sandbox/from-snapshot`.
- Add `DELETE /sandboxes/snapshots` (body or query carries path).
- Update README feature table — snapshot row goes from ❌ to ✅.

### Chunk C — wire-up + integration test

Files: `CubeAPI/examples/snapshot.py` (new), integration test in
`CubeMaster/integration/`.

- Python E2B-SDK example: create → write marker → snapshot → delete →
  restore → verify marker.
- One CI integration test exercising the same path end-to-end.

### Chunk D (deferred to v1.1) — cube-agent quiesce

Out of MVP scope. vsock RPC `PreSnapshot`/`PostSnapshot` in
`agent/src/rpc.rs`, called from Cubelet around capture. Promotes
snapshots from crash-consistent to application-consistent. Largest
chunk; biggest upstream interest.

## Sequencing

1. Chunk A (CubeMaster HTTP).
2. Chunk B (CubeAPI) once A is reachable.
3. Chunk C gates "done."

Upstream issue is optional, not a gate — fork-only is fine for now.

## Mergeability discipline

- Topic branches per chunk: `feature/snapshot-cubemaster`,
  `feature/snapshot-cubeapi`, `feature/snapshot-test`. All branched from
  upstream `main`.
- Rebase weekly against upstream `main`. Cheap if regular.
- No drive-by changes. Spotting something gnarly while in there → file
  an issue, don't fix it.
- Each chunk's commit series PR-shaped from day one: clean messages,
  no fixups, tests where the layer has tests.
- Open the upstream issue before Chunk A. Forcing function for clean diffs.

## Open questions

- **Restore-failure rollback.** Cubelet returns failure → CubeMaster
  returns failure → CubeAPI returns failure → orchestrator never sees a
  half-created sandbox. The new sandbox row in Cube is only written
  after Cubelet success. Confirm this matches existing create-sandbox
  failure semantics during Chunk A.

## Boundary with orchestrator

Orchestrator depends only on these CubeAPI endpoints:

- `POST /sandboxes` (create from `templateID`, or restore via optional
  `fromSnapshot.path`)
- `POST /sandboxes/:sandboxID/snapshots` — body carries
  caller-supplied `snapshotID`
- `DELETE /sandboxes/:sandboxID`
- `DELETE /sandboxes/snapshots` — body or query carries path
- `GET /sandboxes/:sandboxID`
- `POST /sandboxes/:sandboxID/pause` (already works — does serialize to
  disk; not the orchestrator's job to know)
- `POST /sandboxes/:sandboxID/connect` (already works)

Snapshot identity, TTLs, pinning, lineage, reconciliation all live in
the orchestrator. Cube knows nothing about them.
