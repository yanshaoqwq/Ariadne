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

$output = [System.IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
New-Item -ItemType Directory -Force -Path $output | Out-Null
$iscc = (Get-Command ISCC.exe -ErrorAction Stop).Source
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
& $iscc $arguments
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE" }

Get-ChildItem $output -Filter "Ariadne-$($manifest.version)-win-x64-setup.exe" | Select-Object -ExpandProperty FullName
