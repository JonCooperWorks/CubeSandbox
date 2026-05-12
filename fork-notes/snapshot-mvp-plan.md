# Snapshot MVP — fork plan

Fork-local planning doc. Not for upstream merge. Lives under `fork-notes/`
to keep it out of the upstream `docs/` publish path.

## Goal

End-to-end live-VM snapshot + restore through the public CubeAPI HTTP
surface, on a single host, with local-FS blob storage and crash-consistent
semantics. Just enough for an external orchestrator to do:

```
POST   /sandboxes                          { templateID }       -> sandboxID
POST   /sandboxes/:sandboxID/snapshots                          -> snapshotID
DELETE /sandboxes/:sandboxID
POST   /sandboxes                          { snapshotID }       -> sandboxID'
```

Restored sandbox resumes from the captured memory + FS state. Same host only.

## Scope cuts (explicit)

In:

- Live-VM snapshot of a running sandbox (atomic pause + capture).
- Restore from snapshot into a new sandbox on the same host.
- Snapshot as a first-class object with its own UUID, decoupled from the
  source sandbox.
- Local filesystem blob storage, single well-known path.
- Crash-consistent snapshots (no in-guest cooperation).

Out (deferred):

- Cross-host snapshot transfer / shared blob store.
- In-guest quiesce hooks (cube-agent `PreSnapshot` / `PostSnapshot`).
- Application-consistent snapshots.
- Snapshot lineage / parent tracking beyond a flat list.
- Multi-tenancy and authz beyond the existing CubeAPI key check.
- Snapshot GC policies beyond manual delete (orchestrator can manage TTL
  via explicit deletes for now).
- arm64 / non-x86_64 hosts.

## What already exists (don't rebuild)

- **Hypervisor (`hypervisor/vmm/src/api/mod.rs`):** `VmSnapshot` (line 386),
  `VmRestore` (line 389), `VmPauseToSnapshot` (line 320), `VmResumeFromSnapshot`,
  `VmSnapshotConfig` (line 244), `RestoreConfig`. All HTTP-exposed via
  `hypervisor/vmm/src/api/http/http_endpoint.rs`. Snapshot format versioned
  at `SNAPSHOT_VERSION = "1.0.3"` (line 54). Use `VmPauseToSnapshot` —
  it's atomic, do not split into separate pause + snapshot calls.
- **Cubelet AppSnapshot (`Cubelet/services/cubebox/appsnapshot.go:51`):**
  template-build path. Already drives `cube-runtime snapshot`. Pattern to
  mirror, not extend — live-VM snapshot is a different lifecycle.
- **CubeAPI handler scaffold (`CubeAPI/src/handlers/sandboxes.rs:662`):**
  `create_snapshot` route is wired and reaches CubeMaster. Calls
  `state.cubemaster.create_sandbox_snapshot()`. Falls back to a placeholder
  UUID at lines 734–752 when the upstream call returns "endpoint missing."
  **This fallback is the v1 footgun — clients today get a UUID with no
  snapshot behind it.** Removing it is part of the MVP.
- **CubeMaster proto:** `AppSnapshotRequest` exists in the generated
  `api/services/cubebox/v1/cubebox.pb.go` (line 4000+). Confirm whether a
  separate `CreateSandboxSnapshot` proto exists or needs to be added —
  spike #1 below.
- **`cube-runtime`:** drives the actual snapshot capture for AppSnapshot.
  Likely already supports the live-VM case; verify before writing new
  capture code.

## Architecture (one paragraph)

Orchestrator → `CubeAPI` (HTTP) → `CubeMaster` (gRPC) → `Cubelet` (gRPC) →
hypervisor HTTP API → KVM. Snapshot blobs land on the node's local FS at
a configured path. CubeMaster owns the snapshot index (snapshot_id →
node, path, source_sandbox_id, created_at). On restore, CubeMaster picks
the node holding the blob (same node only in v1) and tells Cubelet to
boot a sandbox with `RestoreConfig` pointing at the local path.

## Storage model

Smallest thing that works:

- **On-node path:** `/var/lib/cube/snapshots/<snapshot_id>/` containing the
  Cloud Hypervisor snapshot bundle (memory, devices, config). Hypervisor's
  native format, not a wrapper.
- **Index in CubeMaster:** new table `sandbox_snapshots` with columns
  `snapshot_id, source_sandbox_id, node_id, path, size_bytes, created_at,
  status`. No parent column, no template_version column for v1.
- **Lifecycle:** `pending → ready` on success, `failed` on error. Manual
  delete via `DELETE /sandboxes/snapshots/:id` (new endpoint) removes
  index row + blob directory.
- **No GC.** Orchestrator deletes explicitly. Document the operator's
  responsibility.

Path and base directory configurable via existing CubeMaster config; pick
a sensible default and don't make it pluggable.

## Work breakdown

Five PR-shaped commit series. Each rebases independently against upstream
`main`. Each is individually proposable upstream.

### Chunk A — Cubelet: live-VM snapshot RPC

Files: `Cubelet/services/cubebox/` (new file `livesnapshot.go` mirroring
`appsnapshot.go`), proto in `Cubelet/api/`.

- New gRPC: `CreateSandboxSnapshot(sandbox_id, snapshot_id, snapshot_path)
  -> {size_bytes}`.
- Implementation: resolve sandbox → containerd task → call hypervisor's
  `VmPauseToSnapshot` with `destination_url` set to a `file://` URL pointing
  at `snapshot_path`. Resume the VM after capture (caller's choice via
  flag — default resume).
- New gRPC: `CreateSandboxFromSnapshot(snapshot_id, snapshot_path,
  sandbox_spec) -> {sandbox_id}`. Drives `VmRestore` with `RestoreConfig`.
- New gRPC: `DeleteSandboxSnapshot(snapshot_path)`. Removes the directory.

Don't touch `appsnapshot.go`. The template-snapshot path stays as is.

### Chunk B — CubeMaster: handler + index

Files: `CubeMaster/pkg/service/sandbox/` (new
`sandbox_snapshot.go`), DB migration for the index table.

- HTTP/gRPC handler `CreateSandboxSnapshot`: looks up sandbox → finds
  node → assigns `snapshot_id` + path → calls Cubelet → writes index row.
- Handler `CreateSandboxFromSnapshot`: looks up snapshot → ensures node
  match (same-host only in v1, return error otherwise) → calls Cubelet →
  returns the new `sandboxID`.
- Handler `DeleteSandboxSnapshot`: calls Cubelet to remove blob → deletes
  index row.
- Mirror auth/tenant scoping from existing sandbox endpoints.

### Chunk C — CubeAPI: kill the placeholder, expose restore

Files: `CubeAPI/src/handlers/sandboxes.rs`, `CubeAPI/src/routes.rs`,
`CubeAPI/src/cubemaster/`.

- Remove the `is_endpoint_missing()` fallback at `sandboxes.rs:734–752`.
  Return 503 (or 501) when CubeMaster's snapshot endpoint is genuinely
  unavailable. No more placeholder UUIDs.
- Add `snapshotID` as an optional field on the create-sandbox request
  body. When present, route to a new CubeMaster `CreateSandboxFromSnapshot`
  call instead of the standard create path.
- Add `DELETE /sandboxes/snapshots/:snapshotID`.
- Update the README's feature table — the snapshot row goes from `❌` to
  `✅` after Chunks A–C land together.

### Chunk D — wire-up + integration test

Files: `CubeAPI/examples/snapshot.py` (new), integration test in
`CubeMaster/integration/` or `Cubelet/integration/`.

- Python E2B-SDK example: create sandbox, write a marker file, snapshot,
  delete sandbox, restore from snapshot, verify marker file exists.
- One end-to-end integration test in CI exercising the same path.

### Chunk E (deferred to v1.1) — cube-agent quiesce

Out of MVP scope. Listed here so it's not lost: vsock RPC `PreSnapshot` /
`PostSnapshot` in `agent/src/rpc.rs`, called from Cubelet before/after
the hypervisor capture. Lets in-guest dyson flush SQLite WAL and pause
its agent loop. Promotes snapshots from crash-consistent to
application-consistent. Largest chunk by far; biggest upstream-interest.

## Sequencing

1. Spike #1 below — answer the proto question.
2. Chunk A (Cubelet) and Chunk B (CubeMaster) in parallel once protos are
   settled. They share the proto definition; everything else is independent.
3. Chunk C (CubeAPI) once B's gRPC is reachable.
4. Chunk D (test) gates the chunk-A-through-C series being declared done.
5. Open upstream issue **before** Chunk A. Two reasons: (1) gauge whether
   they have a private branch, (2) lock the proto shape if they have an
   opinion.

## Spikes (do before coding the relevant chunk)

1. **Proto inventory.** Does CubeMaster's proto already define
   `CreateSandboxSnapshot` / `CreateSandboxFromSnapshot` separate from
   `AppSnapshot`? If yes, regen and use. If no, add them. 30 min check.
2. **`cube-runtime` capability.** Does the existing `cube-runtime snapshot`
   binary already work on a live VM, or is it template-only? If it works,
   Chunk A is mostly orchestration; if not, Chunk A grows. Read
   `CubeShim/cube-runtime/` and `CubeShim/shim/src/snapshot/`.
3. **Snapshot bundle layout.** What exactly does
   `VmPauseToSnapshot(destination_url=file://...)` write? One file or a
   directory? Affects index schema (single `path` vs. directory). Read
   `hypervisor/vmm/src/vm.rs` snapshot impl.
4. **Restore on a fresh-spawn VM, same host.** Confirm `VmRestore` works
   when invoked on a hypervisor instance that wasn't the source. Should
   work — it's the whole point — but verify before designing around it.

## Mergeability discipline

- Topic branch per chunk: `feature/snapshot-cubelet`,
  `feature/snapshot-cubemaster`, `feature/snapshot-cubeapi`,
  `feature/snapshot-test`. All branched from upstream `main`.
- Rebase weekly against upstream `main`. Cheap if regular, expensive if
  not.
- No drive-by changes. Spotting something gnarly while in there → file
  an issue, don't fix it. Every unrelated diff is a future merge conflict.
- Each chunk's commit series should be PR-shaped from day one: clean
  messages, no fixups, tests where the layer has tests.
- Open the upstream issue before Chunk A. If they have internal work
  going, fork less. If not, the issue is the forcing function for clean
  diffs.

## Open questions (answer as we go, not blockers)

- Should snapshot delete be eager (synchronous, blocks until blob gone)
  or lazy (mark deleted, sweeper removes later)? Eager for v1; revisit
  if blob removal is slow at scale.
- Should the hypervisor be paused-and-resumed during snapshot, or does
  the orchestrator want a "leave it paused" option? Default resume,
  expose a flag in the gRPC, default false at the CubeAPI surface.
- What's the right error model when restore fails partway? Probably
  CubeMaster rolls back the new sandbox row and returns 500. Define
  precisely during Chunk B.

## Boundary with the Dyson orchestrator

Orchestrator depends only on these CubeAPI endpoints:

- `POST /sandboxes` (create or restore via optional `snapshotID`)
- `POST /sandboxes/:sandboxID/snapshots`
- `DELETE /sandboxes/:sandboxID`
- `DELETE /sandboxes/snapshots/:snapshotID`
- `GET /sandboxes/:sandboxID`
- `POST /sandboxes/:sandboxID/pause` (already works)
- `POST /sandboxes/:sandboxID/connect` (already works)

Anything else stays internal to Cube.
