param(
    [Parameter(Mandatory = $true)][string]$PackageDirectory,
    [string]$OutputDirectory = "artifacts"
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$package = (Resolve-Path $PackageDirectory).Path
$manifest = Get-Content (Join-Path $package "release-manifest.json") -Raw | ConvertFrom-Json
if ($manifest.rid -ne "win-x64") { throw "Windows installer requires win-x64, got $($manifest.rid)" }
$resources = Get-Content (Join-Path $root "core\resources\display_name.json") -Raw | ConvertFrom-Json
$appName = $resources.'ui.brand.name'
if ([string]::IsNullOrWhiteSpace($appName)) { throw "ui.brand.name is missing" }
if ($env:ARIADNE_REQUIRE_SIGNED_RELEASE -eq "1" -and [string]::IsNullOrWhiteSpace($env:ARIADNE_WINDOWS_SIGNTOOL)) {
    throw "formal release requires ARIADNE_WINDOWS_SIGNTOOL"
}

$output = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
}
New-Item -ItemType Directory -Force -Path $output | Out-Null
$isccCommand = Get-Command ISCC.exe -ErrorAction SilentlyContinue
$iscc = if ($null -ne $isccCommand) {
    $isccCommand.Source
} else {
    @(
        (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if ([string]::IsNullOrWhiteSpace($iscc)) { throw "ISCC.exe was not found after installing Inno Setup" }
$arguments = @(
    "/DAppVersion=$($manifest.version)",
    "/DAppName=$appName",
    "/DSourceDir=$package",
    "/DOutputDir=$output"
)
if (-not [string]::IsNullOrWhiteSpace($env:ARIADNE_WINDOWS_SIGNTOOL)) {
    $arguments += "/DSignedBuild=1"
    $arguments += "/Sariadnesign=$($env:ARIADNE_WINDOWS_SIGNTOOL)"
}
$arguments += (Join-Path $PSScriptRoot "Ariadne.iss")
$compilerOutput = & $iscc $arguments 2>&1
$compilerExitCode = $LASTEXITCODE
$compilerOutput | ForEach-Object { Write-Host $_ }
if ($compilerExitCode -ne 0) { throw "Inno Setup failed with exit code $compilerExitCode" }

$installers = @(Get-ChildItem $output -Filter "Ariadne-$($manifest.version)-win-x64-setup.exe")
if ($installers.Count -ne 1) { throw "Inno Setup did not produce exactly one expected installer" }
$installer = $installers[0].FullName
if (-not [string]::IsNullOrWhiteSpace($env:ARIADNE_WINDOWS_SIGNTOOL)) {
    $signature = Get-AuthenticodeSignature -FilePath $installer
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
        throw "Windows installer signature is not valid: $($signature.Status)"
    }
    if ($env:ARIADNE_REQUIRE_SIGNED_RELEASE -eq "1" -and $null -eq $signature.TimeStamperCertificate) {
        throw "formal Windows installer signature is missing a trusted timestamp"
    }
}
Write-Output $installer
