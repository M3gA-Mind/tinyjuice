param(
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string]$Version
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root

$outputDir = 'target/module-e2e'

& (Join-Path $PSScriptRoot 'build-module.ps1') -Crate tinyjuice-module -Version $Version -Target $Target -HostKey ci -OutputDir $outputDir
if ($LASTEXITCODE -ne 0) { throw 'tinyjuice module build failed' }
& (Join-Path $PSScriptRoot 'verify-module.ps1') -Archive "$outputDir/tinyjuice-module-$Version-ci.zip"
if ($LASTEXITCODE -ne 0) { throw 'tinyjuice module end-to-end verification failed' }
