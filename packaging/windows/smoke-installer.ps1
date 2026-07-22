param([Parameter(Mandatory = $true)][string]$Installer)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$timeoutRunner = Join-Path $root "scripts\run-with-timeout.py"
$sandbox = Join-Path $env:TEMP ("ariadne-installer-" + [Guid]::NewGuid().ToString("N"))
$install = Join-Path $sandbox "Program Files\Ariadne"
$env:APPDATA = Join-Path $sandbox "AppData\Roaming"
$userData = Join-Path $env:APPDATA "Ariadne"
New-Item -ItemType Directory -Force -Path $userData | Out-Null
Set-Content -NoNewline -Path (Join-Path $userData "sentinel") -Value "preserve"

function Assert-UserDataPreserved {
    $sentinel = Join-Path (Join-Path $env:APPDATA "Ariadne") "sentinel"
    if ((Get-Content $sentinel -Raw) -ne "preserve") { throw "installer modified product app-state data" }
}

function Invoke-BoundedNative {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @()
    )

    & python3 $timeoutRunner --timeout-seconds $TimeoutSeconds -- $FilePath @Arguments 2>&1 |
        ForEach-Object { Write-Host $_ }
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) { throw "$Label failed: $exitCode" }
}

try {
    $installArguments = @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/DIR=$install")
    Invoke-BoundedNative -Label "first install" -TimeoutSeconds 300 -FilePath $Installer -Arguments $installArguments
    Invoke-BoundedNative -Label "installed layout smoke" -TimeoutSeconds 60 -FilePath (Join-Path $install "Ariadne.Desktop.exe") -Arguments @("--verify-installation")
    Assert-UserDataPreserved

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

    Invoke-BoundedNative -Label "upgrade install" -TimeoutSeconds 300 -FilePath $Installer -Arguments $installArguments
    Invoke-BoundedNative -Label "upgraded layout smoke" -TimeoutSeconds 60 -FilePath (Join-Path $install "Ariadne.Desktop.exe") -Arguments @("--verify-installation")
    Assert-UserDataPreserved

    Invoke-BoundedNative -Label "uninstall" -TimeoutSeconds 300 -FilePath (Join-Path $install "unins000.exe") -Arguments @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")
    Assert-UserDataPreserved
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $sandbox
}
