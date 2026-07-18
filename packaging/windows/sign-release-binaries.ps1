param(
    [Parameter(Mandatory = $true)][string]$DesktopPublishDirectory,
    [Parameter(Mandatory = $true)][string]$RustBinaryDirectory
)

$ErrorActionPreference = "Stop"
$signToolTemplate = $env:ARIADNE_WINDOWS_SIGNTOOL
if ([string]::IsNullOrWhiteSpace($signToolTemplate)) {
    if ($env:ARIADNE_REQUIRE_SIGNED_RELEASE -eq "1") {
        throw "formal release requires ARIADNE_WINDOWS_SIGNTOOL before package assembly"
    }
    return
}
if (-not $signToolTemplate.Contains('$f')) {
    throw 'ARIADNE_WINDOWS_SIGNTOOL must contain the Inno Setup $f file placeholder'
}

$desktop = (Resolve-Path $DesktopPublishDirectory).Path
$rust = (Resolve-Path $RustBinaryDirectory).Path
$binaries = @(
    (Join-Path $desktop "Ariadne.Desktop.exe"),
    (Join-Path $desktop "Ariadne.Desktop.dll"),
    (Join-Path $rust "ariadne.exe"),
    (Join-Path $rust "ariadne-ipc.exe")
)

foreach ($binary in $binaries) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "required Windows release binary is missing before signing: $binary"
    }
    $quoted = '"' + $binary + '"'
    $command = $signToolTemplate.Replace('$f', $quoted)
    & $env:ComSpec /D /S /C $command
    if ($LASTEXITCODE -ne 0) {
        throw "Windows code signing failed for $binary with exit code $LASTEXITCODE"
    }
    $signature = Get-AuthenticodeSignature -FilePath $binary
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
        throw "Windows release binary signature is not valid for $binary: $($signature.Status)"
    }
    if ($env:ARIADNE_REQUIRE_SIGNED_RELEASE -eq "1" -and $null -eq $signature.TimeStamperCertificate) {
        throw "formal Windows release binary signature is missing a trusted timestamp: $binary"
    }
}
