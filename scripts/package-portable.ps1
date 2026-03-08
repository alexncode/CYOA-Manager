param(
  [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$packageJson = Get-Content (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json
$tauriConfig = Get-Content (Join-Path $repoRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json

$productName = [string]$tauriConfig.productName
$version = [string]$packageJson.version
$binaryName = "{0}.exe" -f $packageJson.name
$releaseDir = Join-Path $repoRoot 'src-tauri\target\release'
$sourceExe = Join-Path $releaseDir $binaryName
$sourceViewers = Join-Path $repoRoot 'public\viewers'
$releaseRoot = Join-Path $repoRoot 'release'
$portableRoot = Join-Path $releaseRoot 'portable'
$portableAppDir = Join-Path $portableRoot $productName
$zipPath = Join-Path $releaseRoot ("{0}_{1}_portable_x64.zip" -f $productName, $version)

if (-not $SkipBuild) {
  Push-Location $repoRoot
  try {
    pnpm tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) {
      throw "Portable build failed."
    }
  }
  finally {
    Pop-Location
  }
}

if (-not (Test-Path $sourceExe)) {
  throw "Missing release executable: $sourceExe"
}

if (-not (Test-Path $sourceViewers)) {
  throw "Missing viewers folder: $sourceViewers"
}

New-Item -ItemType Directory -Path $portableRoot -Force | Out-Null

if (Test-Path $portableAppDir) {
  Remove-Item -Path $portableAppDir -Recurse -Force
}

New-Item -ItemType Directory -Path $portableAppDir -Force | Out-Null
Copy-Item -Path $sourceExe -Destination (Join-Path $portableAppDir (Split-Path $sourceExe -Leaf)) -Force
Copy-Item -Path $sourceViewers -Destination (Join-Path $portableAppDir 'viewers') -Recurse -Force

if (Test-Path $zipPath) {
  Remove-Item -Path $zipPath -Force
}

Compress-Archive -Path $portableAppDir -DestinationPath $zipPath -Force

Write-Host "Portable folder:" $portableAppDir
Write-Host "Portable zip:" $zipPath