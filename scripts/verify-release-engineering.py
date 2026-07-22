#!/usr/bin/env python3
import json
import hashlib
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REQUIRED_RIDS = {"linux-x64", "linux-arm64", "win-x64", "osx-x64", "osx-arm64"}
REQUIRED_RUNNERS = {
    "linux-x64": "ubuntu-24.04",
    "linux-arm64": "ubuntu-24.04-arm",
    "win-x64": "windows-2025",
    "osx-x64": "macos-15-intel",
    "osx-arm64": "macos-15",
}
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
    for workflow_job_body, label in ((ci_package, "CI"), (release_package, "Release")):
        for rid, runner in REQUIRED_RUNNERS.items():
            require(
                f"runner: {runner}\n            rid: {rid}" in workflow_job_body,
                f"{label} must run {rid} on its native {runner} runner",
            )
    require("actions/setup-python@v5" in ci_package, "CI native package job must provision Python")
    require("actions/setup-python@v5" in release_gate, "release gate must provision Python")
    require("actions/setup-python@v5" in release_package, "release package job must provision Python")
    for job, label in ((ci_package, "CI"), (release_package, "Release")):
        require("ilammy/msvc-dev-cmd@v1" in job,
                f"{label} Windows package job must initialize the MSVC developer environment")
        require("python3 scripts/run-with-timeout.py --timeout-seconds 600 -- choco install" in job,
                f"{label} Windows packaging-tool install must have a hard timeout")
        require("scripts/run-with-timeout.py --timeout-seconds 1200" in job,
                f"{label} Qdrant provisioning must have a hard timeout")
        require("scripts/run-with-timeout.py --timeout-seconds 300" in job,
                f"{label} Qdrant runtime smoke must have a hard timeout")
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
    dispatch_input = ci.split("full_release_matrix:", 1)[1].split("concurrency:", 1)[0]
    require("default: true" in dispatch_input,
            "manual CI must run the full release matrix unless the caller explicitly opts out")
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
    require('python3 "$ROOT/scripts/run-with-timeout.py"' in build,
            "release build must use the shared process-tree timeout runner")
    for command_timeout in (
        "run_bounded 2700 \"$CARGO_BIN\" build",
        "run_bounded 900 dotnet restore",
        "run_bounded 1200 dotnet publish",
        "run_bounded 600 pwsh",
        "run_bounded 900 bash",
        "run_bounded 600 dotnet run",
    ):
        require(command_timeout in build,
                f"release build is missing bounded execution: {command_timeout}")
    require("--features system-keychain" in build, "formal Rust binaries must use the OS keychain")
    require("--self-contained true" in build, "Desktop release must be self-contained")
    require("verify-package" in build, "release assembly must run package verification")
    require("--bin ariadne-server" not in build, "formal release must not build the REST server")
    require("packaging/windows/sign-release-binaries.ps1" in build,
            "Windows first-party binaries must be signed before package assembly")
    require(build.index("packaging/windows/sign-release-binaries.ps1") < build.index("  assemble "),
            "Windows signing must precede release-manifest assembly")
    require("packaging/macos/sign-release-binaries.sh" in build,
            "macOS Mach-O files must be signed before package assembly")
    require(build.index("packaging/macos/sign-release-binaries.sh") < build.index("  assemble "),
            "macOS nested signing must precede release-manifest assembly")

    release_tool = read("tools/Ariadne.ReleaseTool/Program.cs")
    require('"ariadne-server", "ariadne-server.exe"' in release_tool,
            "package verifier must reject the remote REST server")
    require('new PackageManifest(\n            2,' in release_tool,
            "release manifest must use the explicit platform-seal schema")
    require('new[] { "Ariadne.Desktop" }' in release_tool
            and 'manifest.Rid.StartsWith("osx-"' in release_tool,
            "only the macOS main executable may use platform-sealed integrity")
    require('var verifyManifestBytes = !allowPlatformSealedMutation || !entry.PlatformSealed;' in release_tool
            and 'Package size mismatch' in release_tool
            and 'Package hash mismatch' in release_tool,
            "default package verification must hash every file and post-seal mode may skip only the platform-sealed path")
    require('--allow-platform-sealed-mutation' not in build,
            "pre-seal release assembly must strictly verify every manifest size and SHA-256")
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
    windows_sign = read("packaging/windows/sign-release-binaries.ps1")
    windows_build = read("packaging/windows/build-installer.ps1")
    windows_smoke = read("packaging/windows/smoke-installer.ps1")
    for binary in ("Ariadne.Desktop.exe", "Ariadne.Desktop.dll", "ariadne.exe", "ariadne-ipc.exe"):
        require(binary in windows_sign, f"Windows release signing must cover {binary}")
    require("Get-AuthenticodeSignature" in windows_sign,
            "Windows first-party signatures must be verified after signing")
    require("TimeStamperCertificate" in windows_sign,
            "formal Windows first-party signatures must require a timestamp")
    require("Get-Command ISCC.exe -ErrorAction SilentlyContinue" in windows_build,
            "Windows packaging must tolerate Chocolatey PATH propagation delay")
    require('Inno Setup 6\\ISCC.exe' in windows_build,
            "Windows packaging must probe the fixed Inno Setup install directory")
    require("python3 $timeoutRunner --timeout-seconds 600 -- $iscc @arguments" in windows_build,
            "Windows Inno Setup compilation must use the shared hard timeout")
    require("2>&1 | ForEach-Object { Write-Host $_ }" in windows_build,
            "Windows compiler logs must stream to the host without entering the installer path pipeline")
    require("$installers.Count -ne 1" in windows_build,
            "Windows packaging must require exactly one deterministic installer")
    require("Get-AuthenticodeSignature -FilePath $installer" in windows_build,
            "Windows installer Authenticode signature must be verified")
    require("TimeStamperCertificate" in windows_build,
            "formal Windows installer signature must require a timestamp")
    require("Get-AuthenticodeSignature -FilePath $uninstaller" in windows_smoke,
            "formal Windows smoke must verify the signed uninstaller")
    require("function Invoke-BoundedNative" in windows_smoke
            and windows_smoke.count("Invoke-BoundedNative -Label") == 5,
            "Windows install, verification, upgrade and uninstall must share bounded process supervision")
    require('$env:APPDATA = Join-Path $sandbox "AppData\\Roaming"' in windows_smoke
            and '$userData = Join-Path $env:APPDATA "Ariadne"' in windows_smoke
            and windows_smoke.count("Assert-UserDataPreserved") == 4,
            "Windows smoke sentinel must use the sandboxed product APPDATA/Ariadne path at every stage")

    linux_build = read("packaging/linux/build-deb.sh")
    linux_smoke = read("packaging/linux/smoke-deb.sh")
    require('gpg --batch --verify "$DEB.asc" "$DEB"' in linux_build,
            "Linux package signing must verify the detached signature after creation")
    require('formal release detached signature is missing' in linux_smoke
            and 'gpg --batch --verify "$DEB.asc" "$DEB"' in linux_smoke,
            "formal Linux smoke must require and verify the detached signature")
    require('export XDG_DATA_HOME="$SANDBOX_HOME/xdg-data"' in linux_smoke
            and 'USER_DATA="$XDG_DATA_HOME/Ariadne"' in linux_smoke
            and linux_smoke.count("assert_user_data_preserved") == 4,
            "Linux smoke sentinel must use the sandboxed product XDG_DATA_HOME/Ariadne path at every stage")

    macos_build = read("packaging/macos/build-installer.sh")
    macos_smoke = read("packaging/macos/smoke-installer.sh")
    macos_sign = read("packaging/macos/sign-release-binaries.sh")
    require("file -b \"$candidate\" | grep -q '^Mach-O'" in macos_sign,
            "macOS pre-assembly signing must discover all Mach-O files")
    require("codesign --verify --strict" in macos_sign,
            "macOS nested code signatures must be verified before manifest assembly")
    require('formal release requires ARIADNE_MACOS_SIGNING_IDENTITY before manifest assembly' in macos_sign,
            "formal macOS release must fail before assembly when its signing identity is missing")
    require("realpath" not in macos_build and "realpath" not in macos_smoke,
            "macOS packaging must not depend on non-portable realpath availability")
    require('Ariadne.Desktop" --verify-installation >&2' in macos_build,
            "macOS pre-install smoke output must not contaminate the package path")
    require('pkgbuild "${PKG_ARGS[@]}" "$PKG" >&2' in macos_build,
            "macOS pkgbuild logs must not contaminate the package path")
    require('codesign --verify --deep --strict' in macos_build,
            "macOS app signing must be verified before packaging")
    require('--package "$PACKAGE_DIR" >&2' in macos_build,
            "macOS installer must strictly re-verify staging before copying or sealing it")
    require(macos_build.index('--package "$PACKAGE_DIR" >&2') < macos_build.index('cp -a "$PACKAGE_DIR/."'),
            "strict macOS staging verification must precede app bundle assembly")
    require('codesign --force --deep' not in macos_build,
            "macOS outer app sealing must not rewrite already manifested nested binaries")
    require('--package "$APP/Contents/MacOS"' in macos_build
            and '--allow-platform-sealed-mutation' in macos_build,
            "macOS outer app sealing must explicitly re-verify the assembled manifest in post-seal mode")
    require(macos_build.index('codesign --verify --deep --strict')
            < macos_build.index('--allow-platform-sealed-mutation'),
            "post-seal manifest mutation mode must only run after strict bundle signature verification")
    require('pkgutil --check-signature "$PKG"' in macos_build,
            "macOS signed pkg must be verified before notarization")
    require('xcrun stapler validate "$PKG"' in macos_build
            and 'xcrun stapler validate "$DMG"' in macos_build,
            "macOS notarization tickets must be validated after stapling")
    require("printf '%s\\n' \"$PKG\"" in macos_build,
            "macOS packaging must emit exactly the final pkg path on stdout")
    require('hdiutil attach -readonly -nobrowse -mountpoint' in macos_smoke,
            "macOS smoke must mount the published DMG")
    require('codesign --verify --deep --strict' in macos_smoke,
            "macOS smoke must verify the app from the mounted DMG")
    require('spctl --assess --type install' in macos_smoke
            and 'spctl --assess --type execute' in macos_smoke,
            "formal macOS smoke must assess both pkg and app with Gatekeeper")
    require('python3 "$ROOT/scripts/run-with-timeout.py"' in macos_build
            and macos_build.count("run_bounded ") >= 14,
            "macOS build, signing, packaging and notarization must use bounded process supervision")
    require('python3 "$ROOT/scripts/run-with-timeout.py"' in macos_smoke
            and macos_smoke.count("run_bounded ") >= 13,
            "macOS mount, verification, install and upgrade smoke must use bounded process supervision")

    for workflow, label in ((ci, "CI"), (release, "Release")):
        require("expected installer is missing" in workflow,
                f"{label} must locate the Windows installer by its manifest version")
        require('pkg="artifacts/Ariadne-$version-${{ matrix.rid }}.pkg"' in workflow,
                f"{label} must locate the macOS package by its manifest version and RID")
        require('dmg="artifacts/Ariadne-$version-${{ matrix.rid }}.dmg"' in workflow,
                f"{label} must locate the macOS disk image by its manifest version and RID")
        require('packaging/macos/smoke-installer.sh "$pkg" "$dmg"' in workflow,
                f"{label} must smoke both the macOS pkg and dmg")
        require('$installer = packaging/windows/build-installer.ps1' not in workflow,
                f"{label} must not capture Windows build logs as the installer path")
        require('pkg="$(bash packaging/macos/build-installer.sh' not in workflow,
                f"{label} must not capture macOS build logs as the package path")


def verify_timeout_runner_contract() -> None:
    source = read("scripts/run-with-timeout.py")
    for required in (
        "subprocess.CREATE_NEW_PROCESS_GROUP",
        '["taskkill", "/PID", str(process.pid), "/T", "/F"]',
        'popen_options["start_new_session"] = True',
        "os.killpg(process_group_id, signal.SIGTERM)",
        "os.killpg(process_group_id, signal.SIGKILL)",
        "TIMEOUT_EXIT_CODE = 124",
        "CLEANUP_FAILURE_EXIT_CODE = 125",
        "wait_for_unix_process_group_exit",
    ):
        require(required in source, f"timeout runner is missing process-tree contract: {required}")

    runner = [sys.executable, str(ROOT / "scripts" / "run-with-timeout.py")]
    success = subprocess.run(
        [*runner, "--timeout-seconds", "5", "--", sys.executable, "-c", "print('bounded-ok')"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    require(success.returncode == 0 and "bounded-ok" in success.stdout,
            "timeout runner must preserve successful command output and exit status")

    failure = subprocess.run(
        [*runner, "--timeout-seconds", "5", "--", sys.executable, "-c", "raise SystemExit(7)"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    require(failure.returncode == 7,
            "timeout runner must preserve non-zero command exit status")

    started = time.monotonic()
    timed_out = subprocess.run(
        [*runner, "--timeout-seconds", "0.2", "--", sys.executable, "-c", "import time; time.sleep(30)"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=10,
        check=False,
    )
    elapsed = time.monotonic() - started
    require(timed_out.returncode == 124 and "timed out after 0.2s" in timed_out.stderr,
            "timeout runner must return the stable timeout exit code and diagnostic")
    require(elapsed < 8,
            "timeout runner must terminate the full child process tree promptly")

    if os.name != "nt":
        with tempfile.TemporaryDirectory(prefix="ariadne-timeout-tree-") as temp:
            pid_file = Path(temp) / "pids"
            grandchild = (
                "import os,signal,time,pathlib;"
                "signal.signal(signal.SIGTERM,signal.SIG_IGN);"
                f"pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid()));"
                "time.sleep(30)"
            )
            child = (
                "import os,signal,subprocess,sys,time,pathlib;"
                "signal.signal(signal.SIGTERM,signal.SIG_IGN);"
                f"p=subprocess.Popen([sys.executable,'-c',{grandchild!r}]);"
                f"path=pathlib.Path({str(pid_file)!r});"
                "deadline=time.monotonic()+5;"
                "\nwhile not path.exists() and time.monotonic()<deadline: time.sleep(0.01)\n"
                "path.write_text(str(os.getpid())+' '+path.read_text());"
                "time.sleep(30)"
            )
            parent = (
                "import signal,subprocess,sys,time;"
                f"subprocess.Popen([sys.executable,'-c',{child!r}]);"
                "signal.signal(signal.SIGTERM,lambda *_:sys.exit(0));"
                "time.sleep(30)"
            )
            nested = subprocess.run(
                [*runner, "--timeout-seconds", "1", "--", sys.executable, "-c", parent],
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            require(nested.returncode == 124,
                    "timeout runner must preserve timeout status after nested-tree cleanup")
            pids = [int(value) for value in pid_file.read_text().split()]
            alive = []
            for pid in pids:
                try:
                    os.kill(pid, 0)
                    alive.append(pid)
                except ProcessLookupError:
                    pass
            require(not alive,
                    f"timeout runner returned before nested descendants exited: {alive}")


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
    verify_timeout_runner_contract()
    verify_legal_contract()
    verify_version_consumers(workspace_version())
    print(f"release engineering contract accepted for Ariadne {workspace_version()}")


if __name__ == "__main__":
    main()
