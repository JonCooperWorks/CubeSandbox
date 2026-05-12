# Copyright (c) 2024 Tencent Inc.
# SPDX-License-Identifier: Apache-2.0
#
# End-to-end snapshot test (fork extension).
#
# Demonstrates the runtime-snapshot lifecycle CubeAPI exposes after the
# Chunk A/B commits:
#
#   1. Create a sandbox from a template.
#   2. Write a marker file inside the guest.
#   3. Snapshot the running sandbox.
#   4. Destroy the original sandbox.
#   5. Create a new sandbox by passing `fromSnapshot.path` on create —
#      this restores the captured memory + FS state into a fresh
#      hypervisor instance.
#   6. Verify the marker file is present in the restored sandbox.
#   7. Clean up: destroy the restored sandbox, delete the snapshot blob.
#
# Uses raw HTTP for the snapshot endpoints (orchestrator-facing API,
# not part of the upstream E2B SDK) and the e2b_code_interpreter SDK
# for in-guest code execution.

import os
import sys
import uuid

import httpx
from e2b_code_interpreter import Sandbox

API_URL = os.environ.get("E2B_API_URL", "http://localhost:3000")
API_KEY = os.environ.get("E2B_API_KEY", "dummy")
TEMPLATE_ID = os.environ["CUBE_TEMPLATE_ID"]
MARKER_PATH = "/tmp/snapshot_marker"
MARKER_VALUE = f"hello-from-{uuid.uuid4()}"


def cubeapi() -> httpx.Client:
    return httpx.Client(
        base_url=API_URL,
        headers={"X-API-Key": API_KEY, "Content-Type": "application/json"},
        timeout=60.0,
    )


def create_sandbox(client: httpx.Client, *, from_snapshot_path: str | None = None) -> dict:
    body: dict = {"templateID": TEMPLATE_ID, "timeout": 60}
    if from_snapshot_path is not None:
        body["fromSnapshot"] = {"path": from_snapshot_path}
    r = client.post("/sandboxes", json=body)
    r.raise_for_status()
    return r.json()


def destroy_sandbox(client: httpx.Client, sandbox_id: str) -> None:
    r = client.delete(f"/sandboxes/{sandbox_id}")
    if r.status_code not in (200, 204):
        r.raise_for_status()


def snapshot_sandbox(client: httpx.Client, sandbox_id: str, name: str) -> dict:
    r = client.post(f"/sandboxes/{sandbox_id}/snapshots", json={"name": name})
    r.raise_for_status()
    return r.json()


def delete_snapshot(client: httpx.Client, snapshot_id: str, host_ip: str) -> None:
    r = client.delete(
        f"/sandboxes/snapshots/{snapshot_id}",
        params={"hostIP": host_ip},
    )
    if r.status_code not in (200, 204):
        r.raise_for_status()


def main() -> int:
    snapshot_id = f"snap-{uuid.uuid4()}"
    print(f"snapshot_id = {snapshot_id}")
    print(f"marker      = {MARKER_VALUE}")

    with cubeapi() as api:
        # 1. Create the source sandbox.
        created = create_sandbox(api)
        source_id = created["sandboxID"]
        print(f"created source sandbox {source_id}")

        # 2. Write the marker.
        with Sandbox.connect(source_id) as sb:
            sb.commands.run(f"echo -n {MARKER_VALUE!r} > {MARKER_PATH}")
            check = sb.commands.run(f"cat {MARKER_PATH}")
            assert check.stdout.strip() == MARKER_VALUE, (
                f"marker write failed: got {check.stdout!r}"
            )
            print(f"wrote marker to {MARKER_PATH}")

        # 3. Snapshot.
        snap = snapshot_sandbox(api, source_id, snapshot_id)
        path = snap.get("path")
        host_ip = snap.get("hostIP")
        if not path or not host_ip:
            print(
                "ERROR: snapshot response missing path or hostIP — "
                f"got {snap!r}",
                file=sys.stderr,
            )
            return 1
        print(f"snapshot path    = {path}")
        print(f"snapshot host_ip = {host_ip}")

        # 4. Destroy the original.
        destroy_sandbox(api, source_id)
        print(f"destroyed source sandbox {source_id}")

        # 5. Restore from snapshot.
        restored = create_sandbox(api, from_snapshot_path=path)
        restored_id = restored["sandboxID"]
        print(f"restored sandbox {restored_id} from {path}")

        try:
            # 6. Verify the marker.
            with Sandbox.connect(restored_id) as sb:
                check = sb.commands.run(f"cat {MARKER_PATH}")
                got = check.stdout.strip()
                if got != MARKER_VALUE:
                    print(
                        f"FAIL: marker mismatch after restore: got {got!r}, "
                        f"expected {MARKER_VALUE!r}",
                        file=sys.stderr,
                    )
                    return 1
                print(f"marker survived restore: {got!r}")
        finally:
            # 7a. Drop the restored sandbox.
            destroy_sandbox(api, restored_id)
            print(f"destroyed restored sandbox {restored_id}")

        # 7b. Drop the snapshot blob.
        delete_snapshot(api, snapshot_id, host_ip)
        print(f"deleted snapshot {snapshot_id}")

    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
