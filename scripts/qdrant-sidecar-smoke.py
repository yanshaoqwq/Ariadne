#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
import re
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path


def fail(message: str) -> None:
    raise SystemExit(f"qdrant-smoke: {message}")


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def managed_binary(cache: Path, rid: str) -> tuple[Path, Path]:
    metadata_files = list(cache.resolve().rglob("qdrant-sidecar.json"))
    matches: list[tuple[Path, Path]] = []
    for metadata_path in metadata_files:
        try:
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if metadata.get("rid") != rid:
            continue
        suffix = ".exe" if rid.startswith("win-") else ""
        binary = metadata_path.parent / f"qdrant{suffix}"
        if binary.is_file():
            matches.append((binary, metadata_path))
    if len(matches) != 1:
        fail(f"runtime cache must contain exactly one managed Qdrant for {rid}")
    return matches[0]


def http_json(method: str, url: str, payload: dict | None = None) -> dict:
    body = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        value = json.load(response)
    if not isinstance(value, dict) or value.get("status") != "ok":
        fail(f"Qdrant returned an invalid response for {method} {url}")
    return value


def wait_until_ready(url: str, process: subprocess.Popen[bytes], log_path: Path) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            fail(f"Qdrant exited before readiness ({process.returncode}): {log_path.read_text(errors='replace')[-4000:]}")
        try:
            with urllib.request.urlopen(url, timeout=1) as response:
                if response.status == 200:
                    return
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    fail(f"Qdrant readiness timed out: {log_path.read_text(errors='replace')[-4000:]}")


def main() -> None:
    parser = argparse.ArgumentParser()
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--binary", type=Path)
    source.add_argument("--runtime-cache", type=Path)
    parser.add_argument("--rid", required=True)
    parser.add_argument("--provisioning-evidence", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    if args.runtime_cache is not None:
        binary, metadata_path = managed_binary(args.runtime_cache, args.rid)
        runtime_provisioning = True
    else:
        binary = args.binary.resolve()
        metadata_path = binary.parent / "qdrant-sidecar.json"
        runtime_provisioning = False
    if not binary.is_file() or not metadata_path.is_file():
        fail("Qdrant binary or sidecar metadata is missing")
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    if metadata.get("schema_version") != 1 or metadata.get("product") != "qdrant":
        fail("Qdrant sidecar metadata schema is invalid")
    if metadata.get("rid") != args.rid:
        fail("Qdrant sidecar RID does not match the smoke runner")
    if sha256(binary) != metadata.get("binary_sha256"):
        fail("Qdrant binary SHA-256 does not match sidecar metadata")

    cache_reuse = False
    build_profile = "external"
    if args.provisioning_evidence is not None:
        provisioning = json.loads(args.provisioning_evidence.read_text(encoding="utf-8"))
        if provisioning.get("probe") != "qdrant_runtime_provisioning":
            fail("Qdrant provisioning evidence probe id is invalid")
        if provisioning.get("build_profile") != "release":
            fail("Qdrant provisioning evidence was not generated from a release build")
        if provisioning.get("rid") != args.rid:
            fail("Qdrant provisioning evidence RID does not match the smoke runner")
        if provisioning.get("archive_sha256") != metadata.get("archive_sha256"):
            fail("Qdrant provisioning archive digest does not match runtime metadata")
        if provisioning.get("binary_sha256") != metadata.get("binary_sha256"):
            fail("Qdrant provisioning binary digest does not match runtime metadata")
        runtime_provisioning = provisioning.get("first_use_installed") is True
        cache_reuse = provisioning.get("cache_hit_without_source_archive") is True
        build_profile = "release"

    version_process = subprocess.run(
        [str(binary), "--version"],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    version_text = f"{version_process.stdout}\n{version_process.stderr}"
    version_match = re.search(r"qdrant\s+([0-9]+\.[0-9]+\.[0-9]+)", version_text)
    if version_process.returncode != 0 or not version_match:
        fail(f"Qdrant version probe failed: {version_process.stderr.strip()}")
    if version_match.group(1) != metadata.get("version"):
        fail("Qdrant binary version does not match sidecar metadata")

    http_port = free_port()
    grpc_port = free_port()
    while grpc_port == http_port:
        grpc_port = free_port()
    collection = f"ariadne_release_{uuid.uuid4().hex}"
    clean_shutdown = False
    with tempfile.TemporaryDirectory(prefix="ariadne-qdrant-smoke-") as temporary:
        temporary_path = Path(temporary)
        log_path = temporary_path / "qdrant.log"
        environment = os.environ.copy()
        environment.update({
            "QDRANT__SERVICE__HOST": "127.0.0.1",
            "QDRANT__SERVICE__HTTP_PORT": str(http_port),
            "QDRANT__SERVICE__GRPC_PORT": str(grpc_port),
            "QDRANT__STORAGE__STORAGE_PATH": str(temporary_path / "storage"),
            "QDRANT__STORAGE__SNAPSHOTS_PATH": str(temporary_path / "snapshots"),
        })
        with log_path.open("wb") as log:
            creationflags = (
                subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
            )
            process = subprocess.Popen(
                [str(binary)],
                cwd=temporary_path,
                env=environment,
                stdout=log,
                stderr=subprocess.STDOUT,
                creationflags=creationflags,
            )
            try:
                base = f"http://127.0.0.1:{http_port}"
                wait_until_ready(f"{base}/healthz", process, log_path)
                http_json("PUT", f"{base}/collections/{collection}", {
                    "vectors": {"size": 4, "distance": "Cosine"},
                })
                http_json("PUT", f"{base}/collections/{collection}/points?wait=true", {
                    "points": [
                        {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"marker": "ariadne-release"}},
                        {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"marker": "control"}},
                    ],
                })
                result = http_json("POST", f"{base}/collections/{collection}/points/query", {
                    "query": [1.0, 0.0, 0.0, 0.0],
                    "limit": 1,
                    "with_payload": True,
                })
                points = result.get("result", {}).get("points", [])
                if not points or points[0].get("id") != 1 or points[0].get("payload", {}).get("marker") != "ariadne-release":
                    fail("Qdrant query did not return the inserted release marker")
                http_json("DELETE", f"{base}/collections/{collection}?timeout=30")
            finally:
                if process.poll() is None:
                    if os.name == "nt":
                        process.send_signal(signal.CTRL_BREAK_EVENT)
                    else:
                        process.send_signal(signal.SIGTERM)
                    try:
                        process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
                clean_shutdown = process.returncode == 0

    evidence = {
        "schema_version": 1,
        "probe": "qdrant_sidecar_e2e",
        "build_profile": build_profile,
        "target": {
            "rid": args.rid,
            "qdrant_version": metadata["version"],
            "archive_sha256": metadata["archive_sha256"],
            "binary_sha256": metadata["binary_sha256"],
            "runtime_provisioning": runtime_provisioning,
            "cache_reuse": cache_reuse,
            "index_upsert_search": True,
            "clean_shutdown": clean_shutdown,
        },
    }
    if not clean_shutdown:
        fail("Qdrant did not exit cleanly after SIGTERM")
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Qdrant sidecar E2E accepted for {args.rid}: {output}")


if __name__ == "__main__":
    main()
