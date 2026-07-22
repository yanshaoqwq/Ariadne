#!/usr/bin/env python3
"""Run one native command with a hard deadline and terminate its process tree."""

from __future__ import annotations

import argparse
import os
import signal
import subprocess
import sys
import time
from collections.abc import Sequence


TIMEOUT_EXIT_CODE = 124
CLEANUP_FAILURE_EXIT_CODE = 125


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout-seconds", type=float, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be greater than zero")
    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    if not args.command:
        parser.error("a command is required after --")
    return args


def unix_process_group_exists(process_group_id: int) -> bool:
    try:
        os.killpg(process_group_id, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def wait_for_unix_process_group_exit(
    process: subprocess.Popen[bytes],
    process_group_id: int,
    timeout: float,
) -> bool:
    deadline = time.monotonic() + timeout
    while unix_process_group_exists(process_group_id):
        process.poll()
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.05)
    return True


def terminate_process_tree(process: subprocess.Popen[bytes]) -> None:
    if os.name == "nt":
        if process.poll() is not None:
            return
        try:
            result = subprocess.run(
                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=10,
            )
            if result.returncode not in (0, 128):
                raise RuntimeError(
                    f"taskkill failed while cleaning process tree: {result.returncode}"
                )
        except subprocess.TimeoutExpired as error:
            process.kill()
            raise RuntimeError("taskkill timed out while cleaning process tree") from error
        except OSError as error:
            process.kill()
            raise RuntimeError(f"taskkill could not clean process tree: {error}") from error
        return

    process_group_id = process.pid
    try:
        os.killpg(process_group_id, signal.SIGTERM)
    except ProcessLookupError:
        return

    if not wait_for_unix_process_group_exit(process, process_group_id, 5):
        try:
            os.killpg(process_group_id, signal.SIGKILL)
        except ProcessLookupError:
            return
        if not wait_for_unix_process_group_exit(process, process_group_id, 5):
            raise RuntimeError(
                f"process group {process_group_id} is still alive after SIGKILL"
            )


def normalize_exit_code(return_code: int) -> int:
    if return_code >= 0:
        return return_code
    return min(255, 128 + abs(return_code))


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    creation_flags = 0
    popen_options: dict[str, object] = {}
    if os.name == "nt":
        creation_flags = subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        popen_options["start_new_session"] = True

    try:
        process = subprocess.Popen(
            args.command,
            creationflags=creation_flags,
            **popen_options,
        )
    except OSError as error:
        print(f"bounded command could not start: {error}", file=sys.stderr)
        return 127

    try:
        return normalize_exit_code(process.wait(timeout=args.timeout_seconds))
    except subprocess.TimeoutExpired:
        print(
            f"bounded command timed out after {args.timeout_seconds:g}s: {args.command[0]}",
            file=sys.stderr,
        )
        try:
            terminate_process_tree(process)
        except RuntimeError as error:
            print(f"bounded command cleanup failed: {error}", file=sys.stderr)
            return CLEANUP_FAILURE_EXIT_CODE
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        return TIMEOUT_EXIT_CODE
    except KeyboardInterrupt:
        try:
            terminate_process_tree(process)
        except RuntimeError as error:
            print(f"bounded command cleanup failed: {error}", file=sys.stderr)
            return CLEANUP_FAILURE_EXIT_CODE
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
