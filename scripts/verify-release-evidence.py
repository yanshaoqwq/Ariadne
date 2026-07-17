#!/usr/bin/env python3
import argparse
import json
import re
from pathlib import Path


SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


def load_json(path: Path) -> dict:
    if not path.is_file():
        raise SystemExit(f"release evidence is missing: {path}")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"release evidence is invalid: {path}: {error}") from error
    if not isinstance(value, dict):
        raise SystemExit(f"release evidence must be a JSON object: {path}")
    if value.get("schema_version") != 1:
        raise SystemExit(f"unsupported release evidence schema: {path}")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"release evidence rejected: {message}")


def number(value: object, label: str) -> float:
    require(isinstance(value, (int, float)) and not isinstance(value, bool), f"{label} must be numeric")
    return float(value)


def verify_release_profile(evidence: dict, label: str) -> None:
    require(evidence.get("build_profile") == "release",
            f"{label} evidence was not generated from a release build")


def verify_retrieval(evidence: dict, limits: dict) -> None:
    require(evidence.get("probe") == "million_character_retrieval", "retrieval probe id mismatch")
    verify_release_profile(evidence, "retrieval")
    require(number(evidence.get("character_count"), "retrieval character_count") >= limits["minimum_character_count"],
            "retrieval fixture is smaller than one million characters")
    checks = (
        ("import_ms", "maximum_import_ms"),
        ("initial_index_ms", "maximum_initial_index_ms"),
        ("initial_search_ms", "maximum_search_ms"),
        ("incremental_update_ms", "maximum_incremental_update_ms"),
        ("incremental_search_ms", "maximum_search_ms"),
        ("rebuild_ms", "maximum_rebuild_ms"),
        ("rebuild_search_ms", "maximum_search_ms"),
        ("peak_rss_bytes", "maximum_peak_rss_bytes"),
        ("index_bytes", "maximum_index_bytes"),
    )
    for evidence_key, limit_key in checks:
        require(number(evidence.get(evidence_key), f"retrieval {evidence_key}") <= limits[limit_key],
                f"retrieval {evidence_key} exceeds {limits[limit_key]}")
    for key in ("initial_hits", "incremental_hits", "rebuild_hits"):
        require(number(evidence.get(key), f"retrieval {key}") > 0, f"retrieval {key} must be positive")


def verify_ui(evidence: dict, limits: dict) -> None:
    require(evidence.get("probe") == "desktop_ui_performance", "desktop UI probe id mismatch")
    verify_release_profile(evidence, "desktop UI")
    samples = evidence.get("samples")
    require(isinstance(samples, list), "desktop UI samples must be an array")
    by_count = {sample.get("node_count"): sample for sample in samples if isinstance(sample, dict)}
    for node_count in limits["required_node_counts"]:
        require(node_count in by_count, f"desktop UI evidence is missing {node_count} nodes")
        sample = by_count[node_count]
        require(number(sample.get("frame_count"), f"desktop UI {node_count} frame_count") >=
                limits["minimum_frames_per_node_count"], f"desktop UI {node_count} has too few frames")
        require(number(sample.get("initial_layout_ms"), f"desktop UI {node_count} initial_layout_ms") <=
                limits["maximum_initial_layout_ms"], f"desktop UI {node_count} initial layout exceeds budget")
        require(number(sample.get("p95_frame_interval_ms"),
                       f"desktop UI {node_count} p95_frame_interval_ms") <=
                limits["maximum_p95_frame_interval_ms"],
                f"desktop UI {node_count} p95 frame interval exceeds budget")
        require(number(sample.get("p95_frame_work_ms"),
                       f"desktop UI {node_count} p95_frame_work_ms") <=
                limits["maximum_p95_frame_work_ms"],
                f"desktop UI {node_count} p95 frame work exceeds budget")
        require(number(sample.get("p95_allocated_bytes"), f"desktop UI {node_count} p95_allocated_bytes") <=
                limits["maximum_p95_allocated_bytes"], f"desktop UI {node_count} allocation exceeds budget")


def verify_wcag(evidence: dict, limits: dict) -> None:
    require(evidence.get("probe") == "wcag_contrast", "WCAG probe id mismatch")
    verify_release_profile(evidence, "WCAG")
    require(number(evidence.get("theme_variant_count"), "WCAG theme_variant_count") >=
            limits["minimum_theme_variants"], "WCAG evidence does not cover all theme variants")
    require(number(evidence.get("minimum_normal_text_ratio"), "WCAG normal ratio") >=
            limits["minimum_normal_text_ratio"], "WCAG normal text contrast is below 4.5:1")
    require(number(evidence.get("minimum_large_text_ratio"), "WCAG large ratio") >=
            limits["minimum_large_text_ratio"], "WCAG large text contrast is below 3:1")
    require(number(evidence.get("minimum_non_text_ratio"), "WCAG non-text ratio") >=
            limits["minimum_non_text_ratio"], "WCAG non-text contrast is below 3:1")
    failures = evidence.get("failures")
    require(isinstance(failures, list) and not failures, "WCAG evidence contains failed checks")


def verify_scheduler(evidence: dict, limits: dict) -> None:
    require(evidence.get("probe") == "scheduler_throughput", "scheduler probe id mismatch")
    verify_release_profile(evidence, "scheduler")
    samples = evidence.get("samples")
    require(isinstance(samples, list), "scheduler samples must be an array")
    by_count = {sample.get("node_count"): sample for sample in samples if isinstance(sample, dict)}
    for node_count in limits["required_node_counts"]:
        require(node_count in by_count, f"scheduler evidence is missing {node_count} nodes")
        sample = by_count[node_count]
        require(number(sample.get("sample_count"), f"scheduler {node_count} sample_count") >=
                limits["minimum_samples_per_node_count"], f"scheduler {node_count} has too few samples")
        require(number(sample.get("runnable_width"), f"scheduler {node_count} runnable_width") >=
                (node_count - 1) * limits["minimum_runnable_width_ratio"],
                f"scheduler {node_count} does not exercise a wide runnable queue")
        require(number(sample.get("median_nodes_per_second"),
                       f"scheduler {node_count} median_nodes_per_second") >=
                limits["minimum_median_nodes_per_second"],
                f"scheduler {node_count} throughput is below budget")
    require(number(evidence.get("growth_ratio_500_to_1000"), "scheduler growth ratio") <=
            limits["maximum_growth_ratio_500_to_1000"], "scheduler growth is not near-linear")
    require(number(evidence.get("soak_duration_seconds"), "scheduler soak duration") >=
            limits["minimum_soak_duration_seconds"], "scheduler soak is too short")
    require(number(evidence.get("soak_completed_nodes"), "scheduler soak completed nodes") >=
            limits["minimum_soak_completed_nodes"], "scheduler soak completed too little work")
    require(number(evidence.get("soak_failures"), "scheduler soak failures") == 0,
            "scheduler soak contains failures")
    require(number(evidence.get("peak_rss_bytes"), "scheduler peak_rss_bytes") <=
            limits["maximum_peak_rss_bytes"], "scheduler peak RSS exceeds budget")


def verify_qdrant(evidence_dir: Path, limits: dict, required_rids: set[str]) -> None:
    for rid in sorted(required_rids):
        evidence = load_json(evidence_dir / f"qdrant-sidecar-{rid}.json")
        require(evidence.get("probe") == "qdrant_sidecar_e2e", f"Qdrant {rid} probe id mismatch")
        verify_release_profile(evidence, f"Qdrant {rid}")
        target = evidence.get("target")
        require(isinstance(target, dict), f"Qdrant {rid} target must be an object")
        require(target.get("rid") == rid, f"Qdrant {rid} evidence RID mismatch")
        require(isinstance(target.get("qdrant_version"), str) and target["qdrant_version"],
                f"Qdrant {rid} version is missing")
        for key in ("archive_sha256", "binary_sha256"):
            require(isinstance(target.get(key), str) and SHA256_RE.fullmatch(target[key]) is not None,
                    f"Qdrant {rid} {key} is not a SHA-256 digest")
        if limits["require_runtime_provisioning"]:
            require(target.get("runtime_provisioning") is True,
                    f"Qdrant {rid} runtime provisioning failed")
        if limits["require_cache_reuse"]:
            require(target.get("cache_reuse") is True, f"Qdrant {rid} cache reuse failed")
        if limits["require_index_upsert_search"]:
            require(target.get("index_upsert_search") is True, f"Qdrant {rid} retrieval E2E failed")
        if limits["require_clean_shutdown"]:
            require(target.get("clean_shutdown") is True, f"Qdrant {rid} did not shut down cleanly")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=Path(__file__).resolve().parents[1], type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    parser.add_argument("--rid", action="append", choices=(
        "linux-x64", "linux-arm64", "win-x64", "osx-x64", "osx-arm64",
    ))
    args = parser.parse_args()
    root = args.root.resolve()
    evidence_dir = (args.evidence_dir or root / "artifacts" / "release-evidence").resolve()
    limits = load_json(root / "packaging" / "release-performance.json")
    matrix = load_json(root / "packaging" / "release-matrix.json")

    verify_retrieval(load_json(evidence_dir / "million-character-retrieval.json"),
                     limits["million_character_retrieval"])
    verify_ui(load_json(evidence_dir / "desktop-ui-performance.json"),
              limits["desktop_ui_performance"])
    verify_wcag(load_json(evidence_dir / "wcag-contrast.json"), limits["wcag_contrast"])
    verify_scheduler(load_json(evidence_dir / "scheduler-throughput.json"),
                     limits["scheduler_throughput"])
    matrix_rids = {target["rid"] for target in matrix["targets"]}
    required_rids = set(args.rid) if args.rid else matrix_rids
    require(required_rids <= matrix_rids, "Qdrant evidence requested an unsupported RID")
    verify_qdrant(evidence_dir, limits["qdrant_sidecar_e2e"], required_rids)
    print(f"release evidence accepted: {evidence_dir}")


if __name__ == "__main__":
    main()
