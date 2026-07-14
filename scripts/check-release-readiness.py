#!/usr/bin/env python3
import argparse
import json
import re
from pathlib import Path


def workspace_version(root: Path) -> str:
    content = (root / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'(?m)^version\s*=\s*"([^"]+)"', content)
    if not match:
        raise SystemExit("workspace.package.version is missing")
    return match.group(1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=Path(__file__).resolve().parents[1], type=Path)
    parser.add_argument("--tag")
    args = parser.parse_args()
    root = args.root.resolve()
    state = json.loads((root / "packaging" / "release-readiness.json").read_text(encoding="utf-8"))
    blockers = state.get("open_blockers") or []
    if not state.get("release_ready") or blockers:
        details = ", ".join(blockers) if blockers else "release_ready=false"
        raise SystemExit(f"release gate rejected: {details}")
    if args.tag:
        expected = f"v{workspace_version(root)}"
        if args.tag != expected:
            raise SystemExit(f"release tag {args.tag!r} does not match {expected!r}")
    print(f"release gate accepted for Ariadne {workspace_version(root)}")


if __name__ == "__main__":
    main()
