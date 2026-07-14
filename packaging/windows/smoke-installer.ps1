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
