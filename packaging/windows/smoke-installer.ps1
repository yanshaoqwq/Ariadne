param([Parameter(Mandatory = $true)][string]$Installer)

$ErrorActionPreference = "Stop"
$sandbox = Join-Path $env:TEMP ("ariadne-installer-" + [Guid]::NewGuid().ToString("N"))
$install = Join-Path $sandbox "Program Files\Ariadne"
$userData = Join-Path $sandbox "UserData"
New-Item -ItemType Directory -Force -Path $userData | Out-Null
Set-Content -NoNewline -Path (Join-Path $userData "sentinel") -Value "preserve"
try {
    & $Installer /VERYSILENT /SUPPRESSMSGBOXES /NORESTART "/DIR=$install"
    if ($LASTEXITCODE -ne 0) { throw "first install failed: $LASTEXITCODE" }
    & (Join-Path $install "Ariadne.Desktop.exe") --verify-installation
    if ($LASTEXITCODE -ne 0) { throw "installed layout smoke failed: $LASTEXITCODE" }

    if ($env:ARIADNE_REQUIRE_SIGNED_RELEASE -eq "1") {
        $uninstaller = Join-Path $install "unins000.exe"
        $signature = Get-AuthenticodeSignature -FilePath $uninstaller
        if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
            throw "Windows uninstaller signature is not valid: $($signature.Status)"
        }
        if ($null -eq $signature.TimeStamperCertificate) {
            throw "formal Windows uninstaller signature is missing a trusted timestamp"
        }
    }

    & $Installer /VERYSILENT /SUPPRESSMSGBOXES /NORESTART "/DIR=$install"
    if ($LASTEXITCODE -ne 0) { throw "upgrade install failed: $LASTEXITCODE" }
    if ((Get-Content (Join-Path $userData "sentinel") -Raw) -ne "preserve") { throw "upgrade removed user data" }

    & (Join-Path $install "unins000.exe") /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
    if ($LASTEXITCODE -ne 0) { throw "uninstall failed: $LASTEXITCODE" }
    if ((Get-Content (Join-Path $userData "sentinel") -Raw) -ne "preserve") { throw "uninstall removed user data" }
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $sandbox
}
