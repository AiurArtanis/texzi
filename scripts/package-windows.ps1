param([string] $OutputDirectory = 'dist')

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
Set-StrictMode -Version Latest
Set-Location (Split-Path $PSScriptRoot -Parent)

. "$PSScriptRoot/windows-package.ps1"
. "$PSScriptRoot/package-licenses.ps1"

$commit = (git rev-parse HEAD).Trim()
if ($env:GITHUB_SHA -and $commit -ne $env:GITHUB_SHA) { throw 'Checkout SHA mismatch.' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a new package output directory.' }
$runId = $env:GITHUB_RUN_ID ?? 'local'
$runAttempt = $env:GITHUB_RUN_ATTEMPT ?? 'local'
$sourceExe = "target/$windowsTarget/release/texzi-app.exe"
$package = Join-Path $OutputDirectory 'package'
$licenses = Join-Path $package 'licenses'
New-Item -Path $licenses -ItemType Directory | Out-Null

Copy-Item -LiteralPath $sourceExe, 'LICENSE' -Destination $package
Copy-Item -LiteralPath 'docs/WINDOWS_RUN.md' -Destination (Join-Path $package 'RUN.md')
Copy-Item -LiteralPath 'crates/texzi-app/assets/README.md' -Destination (Join-Path $licenses 'ink-z-provenance.md')
Copy-Item -LiteralPath 'docs/t05/LICENSE-SOURCES.json' -Destination (Join-Path $licenses 'supplemental-rich-sources.json')
Copy-Item -LiteralPath 'docs/tx00/licenses' -Destination (Join-Path $licenses 'supplemental') -Recurse
$metadata = (cargo metadata --locked --format-version 1 --filter-platform $windowsTarget) | ConvertFrom-Json
Copy-DependencyLicenses -Metadata $metadata -Destination $licenses -ProjectLicense LICENSE | Out-Null
$sysroot = (rustc --print sysroot).Trim()
Copy-Item -LiteralPath (Join-Path $sysroot 'share/doc/rust') -Destination (Join-Path $licenses 'rust-runtime') -Recurse

@(
    "commit=$commit"
    "target=$windowsTarget"
    "run=$runId"
    "attempt=$runAttempt"
    "image=$env:ImageOS/$env:ImageVersion"
    "rust=$(rustc --version)"
    "msvc=$($env:VCToolsVersion)"
    "sdk=$(($env:WindowsSDKVersion ?? '').TrimEnd('\'))"
) | Set-Content -LiteralPath (Join-Path $package 'build-identity.txt') -Encoding ascii
$exeHash = (Get-FileHash -LiteralPath (Join-Path $package 'texzi-app.exe') -Algorithm SHA256).Hash
"$exeHash  texzi-app.exe" | Set-Content -LiteralPath (Join-Path $package 'SHA256SUMS.txt') -Encoding ascii

$zipName = "texzi-$($commit.Substring(0, 7))-win-x64.zip"
$zipPath = Join-Path $OutputDirectory $zipName
Compress-Archive -Path (Join-Path $package '*') -DestinationPath $zipPath
$zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
"$zipHash  $zipName" | Set-Content -LiteralPath (Join-Path $OutputDirectory 'SHA256SUMS.txt') -Encoding ascii
Confirm-WindowsCandidate -Directory $OutputDirectory -Commit $commit -RunId $runId -RunAttempt $runAttempt
