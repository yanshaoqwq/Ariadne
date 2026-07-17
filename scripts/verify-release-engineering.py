#!/usr/bin/env python3
import json
import hashlib
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REQUIRED_RIDS = {"linux-x64", "linux-arm64", "win-x64", "osx-x64", "osx-arm64"}
REQUIRED_QUALITY_COMMANDS = {
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo test --workspace --all-targets --all-features --locked",
    "dotnet test desktop/Ariadne.slnx --configuration Release --no-restore",
}
ALLOWED_STATIC_BLOCKERS = {"LEGAL_REVIEW"}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"release engineering contract rejected: {message}")


def read(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def read_json(relative: str) -> dict:
    value = json.loads(read(relative))
    require(isinstance(value, dict), f"{relative} must contain a JSON object")
    return value


def workspace_version() -> str:
    match = re.search(
        r'(?ms)^\[workspace\.package\]\s*.*?^version\s*=\s*"([^"]+)"',
        read("Cargo.toml"),
    )
    require(match is not None, "Cargo.toml must define workspace.package.version")
    return match.group(1)


def workflow_job(workflow: str, job: str) -> str:
    lines = workflow.splitlines()
    marker = f"  {job}:"
    try:
        start = lines.index(marker)
    except ValueError as error:
        raise SystemExit(f"release engineering contract rejected: missing workflow job {job}") from error
    end = len(lines)
    for index in range(start + 1, len(lines)):
        if re.fullmatch(r"  [a-zA-Z0-9_-]+:", lines[index]):
            end = index
            break
    return "\n".join(lines[start:end])


def verify_toolchain_and_quality(ci: str, release: str) -> None:
    toolchain = read("rust-toolchain.toml")
    require('channel = "1.96.0"' in toolchain, "Rust toolchain must be pinned to 1.96.0")
    require('components = ["clippy", "rustfmt"]' in toolchain, "toolchain must include clippy and rustfmt")
    for workflow, label in ((ci, "CI"), (release, "Release")):
        quality = workflow_job(workflow, "quality")
        missing = sorted(command for command in REQUIRED_QUALITY_COMMANDS if command not in quality)
        require(not missing, f"{label} quality job is missing: {', '.join(missing)}")
        require("python3 scripts/verify-release-engineering.py" in quality, f"{label} quality job must verify this contract")
        require("EmbarkStudios/cargo-deny-action@v2" in quality, f"{label} must run cargo-deny")
        require("licenses --root . --output /tmp/THIRD_PARTY_NOTICES.md" in quality, f"{label} must regenerate third-party notices")
        require("actions/setup-python@v5" in quality, f"{label} quality job must provision Python")


def verify_release_matrix(ci: str, release: str) -> None:
    matrix = read_json("packaging/release-matrix.json")
    rids = {target.get("rid") for target in matrix.get("targets", [])}
    require(rids == REQUIRED_RIDS, "packaging release RID matrix is incomplete")
    for rid in REQUIRED_RIDS:
        require(rid in workflow_job(ci, "native-package"), f"CI native package matrix is missing {rid}")
        require(rid in workflow_job(release, "package"), f"release package matrix is missing {rid}")
    ci_package = workflow_job(ci, "native-package")
    release_gate = workflow_job(release, "gate")
    release_package = workflow_job(release, "package")
    release_evidence_gate = workflow_job(release, "evidence-gate")
    release_publish = workflow_job(release, "publish")
    require("actions/setup-python@v5" in ci_package, "CI native package job must provision Python")
    require("actions/setup-python@v5" in release_gate, "release gate must provision Python")
    require("actions/setup-python@v5" in release_package, "release package job must provision Python")
    require("scripts/check-release-readiness.py" in release_gate, "tag workflow must execute the readiness gate")
    require("--static-only" in release_gate, "tag preflight must validate static blockers before expensive jobs")
    require("--tag" in release_gate, "tag workflow must validate the release tag")
    require("check-release-readiness.py" in release_evidence_gate,
            "tag workflow must run the final readiness and evidence gate")
    require("--evidence-dir" in release_evidence_gate,
            "tag final gate must validate freshly generated evidence")
    require("needs: [package, evidence-gate]" in release_publish,
            "release publishing must depend on package and evidence gates")
    require("ARIADNE_REQUIRE_SIGNED_RELEASE" in release_package, "tag packages must require signing")
    for job, label in ((ci_package, "CI native package"), (release_package, "release package")):
        for command in (
            "scripts/build-release.sh",
            "packaging/linux/smoke-deb.sh",
            "packaging/windows/smoke-installer.ps1",
            "packaging/macos/smoke-installer.sh",
        ):
            require(command in job, f"{label} job must execute {command}")
        require("qdrant_runtime_provisioning_installs_then_uses_cache" in job,
                f"{label} job must provision and reuse managed Qdrant")
        require("scripts/qdrant-sidecar-smoke.py" in job,
                f"{label} job must execute the Qdrant runtime smoke probe")
    for target in matrix["targets"]:
        signing_input = target.get("signing_input")
        require(isinstance(signing_input, str) and signing_input in release, f"release workflow is missing signing input for {target['rid']}")
        for notarization_input in target.get("notarization_inputs", []):
            require(notarization_input in release, f"release workflow is missing {notarization_input}")

    qdrant = read_json("packaging/qdrant-sidecars.json")
    qdrant_targets = {target.get("rid"): target for target in qdrant.get("targets", [])}
    require(set(qdrant_targets) == REQUIRED_RIDS, "Qdrant sidecar RID matrix is incomplete")
    for rid, target in qdrant_targets.items():
        digest = target.get("archive_sha256")
        require(
            isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest) is not None,
            f"Qdrant sidecar {rid} must pin the official archive SHA-256",
        )


def verify_ci_execution_policy(ci: str, release: str) -> None:
    require("full_release_matrix:" in ci, "CI manual full release matrix input is missing")
    require("cancel-in-progress: true" in ci, "CI must cancel superseded branch runs")
    manual_gate = "github.event_name == 'workflow_dispatch' && inputs.full_release_matrix"
    for job in ("native-package", "performance-evidence", "evidence-gate"):
        body = workflow_job(ci, job)
        require(manual_gate in body, f"CI {job} must be gated behind the manual full release matrix")
        require("timeout-minutes:" in body, f"CI {job} must have a timeout")
    require("timeout-minutes:" in workflow_job(ci, "quality"), "CI quality job must have a timeout")
    for job in ("gate", "quality", "package", "performance-evidence", "evidence-gate", "publish"):
        require("timeout-minutes:" in workflow_job(release, job), f"release {job} job must have a timeout")
    require("cancel-in-progress: false" in release, "release workflow must not cancel an active tag release")


def verify_readiness_contract() -> None:
    readiness = read_json("packaging/release-readiness.json")
    blockers = readiness.get("open_blockers")
    require(isinstance(blockers, list), "release readiness blockers must be a list")
    require(all(isinstance(blocker, str) and blocker for blocker in blockers),
            "release readiness blockers must be non-empty strings")
    require(len(blockers) == len(set(blockers)), "release readiness blockers must be unique")
    unexpected = sorted(set(blockers) - ALLOWED_STATIC_BLOCKERS)
    require(not unexpected,
            "runtime-generated evidence cannot be a static blocker: " + ", ".join(unexpected))
    require(readiness.get("release_ready") == (not blockers),
            "release_ready must exactly reflect the static blocker list")


def verify_package_security_contract() -> None:
    build = read("scripts/build-release.sh")
    require("--features system-keychain" in build, "formal Rust binaries must use the OS keychain")
    require("--self-contained true" in build, "Desktop release must be self-contained")
    require("verify-package" in build, "release assembly must run package verification")
    require("--bin ariadne-server" not in build, "formal release must not build the REST server")

    release_tool = read("tools/Ariadne.ReleaseTool/Program.cs")
    require('"ariadne-server", "ariadne-server.exe"' in release_tool,
            "package verifier must reject the remote REST server")
    desktop_validator = read("desktop/Ariadne.Desktop/ReleaseLayoutValidator.cs")
    require("release package must not contain the remote REST server" in desktop_validator,
            "Desktop installation probe must reject the remote REST server")
    backend_client = read("desktop/Ariadne.Desktop/Backend/JsonLineBackendClient.cs")
    require('Path.Combine("Backend", executableName)' in backend_client,
            "Desktop must discover the packaged backend relative to its application directory")

    secrets = read("core/src/config/secrets.rs")
    require("Algorithm::Argon2id" in secrets and "getrandom::getrandom" in secrets,
            "local secret fallback must use Argon2id with random salt/nonce")
    require("system keychain is unavailable; set a local secret master password" in secrets,
            "local secret fallback must fail closed without a master password")
    require("ARIADNE_ALLOW_LEGACY_MACHINE_SECRET_MIGRATION" in secrets,
            "legacy machine-bound secrets must require explicit migration mode")
    require("derive_local_secret_key" not in secrets,
            "predictable machine-derived secret fallback must not return to production")

    rest = read("core/src/rest.rs")
    require("REST bind address must be loopback; remote plaintext HTTP is not supported" in rest,
            "REST server must reject non-loopback bind addresses")
    require("MAX_HTTP_HEADER_BYTES" in rest and "MAX_HTTP_BODY_BYTES" in rest and "HTTP_IO_TIMEOUT" in rest,
            "loopback REST compatibility server must retain bounded HTTP input and IO")


def verify_installer_smoke_contract(ci: str, release: str) -> None:
    windows_build = read("packaging/windows/build-installer.ps1")
    require("Get-Command ISCC.exe -ErrorAction SilentlyContinue" in windows_build,
            "Windows packaging must tolerate Chocolatey PATH propagation delay")
    require('Inno Setup 6\\ISCC.exe' in windows_build,
            "Windows packaging must probe the fixed Inno Setup install directory")
    require("$compilerOutput = & $iscc $arguments 2>&1" in windows_build,
            "Windows packaging must isolate compiler logs from its output value")
    require("$compilerOutput | ForEach-Object { Write-Host $_ }" in windows_build,
            "Windows compiler logs must not enter the installer path pipeline")
    require("$installers.Count -ne 1" in windows_build,
            "Windows packaging must require exactly one deterministic installer")

    macos_build = read("packaging/macos/build-installer.sh")
    macos_smoke = read("packaging/macos/smoke-installer.sh")
    require("realpath" not in macos_build and "realpath" not in macos_smoke,
            "macOS packaging must not depend on non-portable realpath availability")
    require('Ariadne.Desktop" --verify-installation >&2' in macos_build,
            "macOS pre-install smoke output must not contaminate the package path")
    require('pkgbuild "${PKG_ARGS[@]}" "$PKG" >&2' in macos_build,
            "macOS pkgbuild logs must not contaminate the package path")
    require("printf '%s\\n' \"$PKG\"" in macos_build,
            "macOS packaging must emit exactly the final pkg path on stdout")

    for workflow, label in ((ci, "CI"), (release, "Release")):
        require("expected installer is missing" in workflow,
                f"{label} must locate the Windows installer by its manifest version")
        require('pkg="artifacts/Ariadne-$version-${{ matrix.rid }}.pkg"' in workflow,
                f"{label} must locate the macOS package by its manifest version and RID")
        require('$installer = packaging/windows/build-installer.ps1' not in workflow,
                f"{label} must not capture Windows build logs as the installer path")
        require('pkg="$(bash packaging/macos/build-installer.sh' not in workflow,
                f"{label} must not capture macOS build logs as the package path")


def verify_legal_contract() -> None:
    legal = read_json("packaging/release-legal.json")
    require(legal.get("schema_version") == 1, "unsupported release legal schema")
    expression = legal.get("license_expression")
    name = legal.get("license_name")
    notice = legal.get("required_notice")
    require(isinstance(expression, str) and expression, "legal license_expression is missing")
    require(isinstance(name, str) and name, "legal license_name is missing")
    require(isinstance(notice, str) and notice.startswith("Required Notice: "), "legal required_notice is invalid")

    license_bytes = (ROOT / "LICENSE").read_bytes()
    require(
        hashlib.sha256(license_bytes).hexdigest() == legal.get("license_sha256"),
        "LICENSE changed without updating the reviewed legal manifest",
    )
    license_text = license_bytes.decode("utf-8")
    require(license_text.startswith(f"{notice}\n\n# {name}\n"), "LICENSE notice or title does not match the legal manifest")
    for heading in (
        "## Acceptance",
        "## Copyright License",
        "## Distribution License",
        "## Notices",
        "## Changes and New Works License",
        "## Patent License",
        "## Noncommercial Purposes",
        "## Personal Uses",
        "## Noncommercial Organizations",
        "## Fair Use",
        "## No Other Rights",
        "## Patent Defense",
        "## Violations",
        "## No Liability",
        "## Definitions",
    ):
        require(license_text.count(heading) == 1, f"LICENSE canonical section is missing or duplicated: {heading}")

    required_files = legal.get("required_release_files")
    require(isinstance(required_files, list) and required_files, "legal required_release_files is missing")
    for relative in required_files:
        require(isinstance(relative, str) and (ROOT / relative).is_file(), f"required legal file is missing: {relative}")
    required_literal = '"LICENSE", "NOTICE", "COMMERCIAL_LICENSE.md", "THIRD_PARTY_NOTICES.md"'
    require(required_literal in read("tools/Ariadne.ReleaseTool/Program.cs"), "ReleaseTool must package every legal file")
    desktop_project = read("desktop/Ariadne.Desktop/Ariadne.Desktop.csproj")
    for relative in required_files:
        require(relative in desktop_project, f"Desktop publish must include {relative}")

    cargo = read("Cargo.toml")
    require(f'license = "{expression}"' in cargo, "Cargo workspace license does not match the legal manifest")
    require("license.workspace = true" in read("core/Cargo.toml"), "core crate must inherit the workspace license")
    require(f"<PackageLicenseExpression>{expression}</PackageLicenseExpression>" in desktop_project,
            ".NET license does not match the legal manifest")
    require(read("NOTICE").startswith(f"{notice}\n"), "NOTICE does not preserve the required notice")

    readme = read("README.md")
    for marker in (name, "source-available", "不是 OSI", "COMMERCIAL_LICENSE.md", "THIRD_PARTY_NOTICES.md", "CLA.md"):
        require(marker in readme, f"README legal disclosure is missing: {marker}")
    commercial = read("COMMERCIAL_LICENSE.md")
    require("不构成商业许可要约" in commercial and "不修改 `LICENSE`" in commercial,
            "commercial license contact file must not grant or modify rights")
    cla_path = legal.get("contributor_license_agreement")
    acceptance = legal.get("cla_acceptance")
    require(isinstance(cla_path, str) and (ROOT / cla_path).is_file(), "CLA file is missing")
    require(isinstance(acceptance, str) and acceptance in read(cla_path), "CLA acceptance text is missing from the agreement")
    require(acceptance in read(".github/PULL_REQUEST_TEMPLATE.md"), "pull request template must record CLA acceptance")

    review = legal.get("legal_review")
    require(isinstance(review, dict), "legal review state is missing")
    blocker = review.get("release_blocker")
    readiness = read_json("packaging/release-readiness.json")
    open_blockers = readiness.get("open_blockers") or []
    if review.get("status") == "pending":
        require(blocker in open_blockers, "pending legal review must block release readiness")
        require(review.get("approval_reference") is None, "pending legal review cannot carry an approval reference")
    elif review.get("status") == "approved":
        require(isinstance(review.get("approval_reference"), str) and review["approval_reference"].strip(),
                "approved legal review requires an auditable approval reference")
        require(blocker not in open_blockers, "approved legal review cannot remain an open blocker")
    else:
        require(False, "legal review status must be pending or approved")


def verify_version_consumers(version: str) -> None:
    require("version.workspace = true" in read("core/Cargo.toml"), "core crate must inherit workspace version")
    require('env!("CARGO_PKG_VERSION")' in read("core/src/lib.rs"), "Rust product version must come from Cargo")
    require('"product_version": crate::PRODUCT_VERSION' in read("core/src/cli.rs"), "CLI must expose product version")
    require('"product_version": crate::PRODUCT_VERSION' in read("core/src/ipc.rs"), "IPC hello must expose product version")
    props = read("Directory.Build.props")
    require("[workspace\\.package\\]" in props, ".NET version parser must target workspace.package")
    require("<Version>$(AriadneProductVersion)</Version>" in props, ".NET Version must use AriadneProductVersion")
    require("<AssemblyVersion>$(AriadneProductVersion).0</AssemblyVersion>" in props, ".NET AssemblyVersion must use AriadneProductVersion")
    release_tool = read("tools/Ariadne.ReleaseTool/Program.cs")
    require("[workspace\\.package\\]" in release_tool, "release tool must parse workspace.package.version")
    require("Desktop version {version} does not match Cargo workspace version" in release_tool, "release assembly must reject version drift")
    require(version == workspace_version(), "workspace version changed during validation")


def main() -> None:
    ci = read(".github/workflows/ci.yml")
    release = read(".github/workflows/release.yml")
    verify_toolchain_and_quality(ci, release)
    verify_release_matrix(ci, release)
    verify_ci_execution_policy(ci, release)
    verify_readiness_contract()
    verify_package_security_contract()
    verify_installer_smoke_contract(ci, release)
    verify_legal_contract()
    verify_version_consumers(workspace_version())
    print(f"release engineering contract accepted for Ariadne {workspace_version()}")


if __name__ == "__main__":
    main()
