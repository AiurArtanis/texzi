param([switch]$FailureProbe)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Set-Location (Split-Path $PSScriptRoot -Parent)
if (Test-Path dist) { throw 'dist already exists; use a clean checkout to avoid stale candidates.' }
New-Item build-evidence -ItemType Directory -Force | Out-Null
if (Test-Path build-evidence/build.log) { throw 'build-evidence/build.log already exists; preserve it and use a clean checkout.' }
. "$PSScriptRoot/native-command.ps1"
Start-Transcript -Path build-evidence/transcript.log
try {
    if ($FailureProbe) {
        Invoke-Checked (Join-Path $PSHOME 'pwsh.exe') @('-NoProfile', '-Command', '[Console]::Out.WriteLine("TX00_FAILURE_STDOUT"); [Console]::Error.WriteLine("TX00_FAILURE_STDERR"); exit 23')
        throw 'Failure probe unexpectedly continued'
    }
    Invoke-Checked git @('rev-parse', 'HEAD') -StdoutFile 'build-evidence/checkout-commit.txt'
    $commit = (Get-Content build-evidence/checkout-commit.txt -Raw).Trim()
    if ($env:GITHUB_SHA -and $commit -ne $env:GITHUB_SHA) { throw 'Checkout SHA mismatch' }
    Get-ComputerInfo | Select-Object WindowsProductName, WindowsVersion, OsBuildNumber, OsArchitecture | Format-List
    Write-Host "commit=$commit run=$env:GITHUB_RUN_ID attempt=$env:GITHUB_RUN_ATTEMPT image=$env:ImageOS/$env:ImageVersion"
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    Invoke-Checked $vswhere @('-latest', '-products', '*', '-format', 'json')
    $inventory = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\Lib" | Select-Object -ExpandProperty Name
    $inventory | Set-Content build-evidence/installed-sdk-directories.txt
    Invoke-Checked $vswhere @('-latest', '-products', '*', '-property', 'installationPath') -StdoutFile 'build-evidence/vs-installation.txt'
    $vs = (Get-Content build-evidence/vs-installation.txt -Raw).Trim()
    $defaultMsvc = Get-Content (Join-Path $vs 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt')
    @{
        installed_sdk_directories=$inventory; default_msvc_file_version=$defaultMsvc;
        selection_boundary='Inventory/default only. This file does not prove selection. See effective-toolchain-config.json and final-app-link-command.txt for this build final-link evidence; C compiler/header and OS loader selection are not traced.'
    } | ConvertTo-Json -Depth 4 | Set-Content build-evidence/tool-inventory.json
    . "$PSScriptRoot/select-linker.ps1"
    Invoke-Checked rustup @('show')
    Invoke-Checked rustc @('-Vv')
    Invoke-Checked cargo @('-V')
    $manifest = Get-Content docs/design-v1/manifest-sha256.json -Raw | ConvertFrom-Json
    foreach ($entry in $manifest.files) {
        $file = Join-Path 'docs/design-v1' $entry.path
        if (!(Test-Path -LiteralPath $file)) { throw "Missing design file: $file" }
        if ((Get-Item -LiteralPath $file).Length -ne $entry.bytes -or
            (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $entry.sha256) {
            throw "Design manifest mismatch: $file"
        }
    }
    Write-Host "Design manifest: $($manifest.files.Count) entries verified"
    Invoke-Checked cargo @('fmt', '--all', '--', '--check')
    Invoke-Checked cargo @('tree', '--locked', '--target', 'x86_64-pc-windows-msvc', '-e', 'features')
    Write-Host 'Component tests: NOT_RUN (TX-00 adds no testable document logic; native smoke is separate).'
    Invoke-Checked cargo @('build', '--release', '--locked', '--target', 'x86_64-pc-windows-msvc')
    $buildResult = Get-ChildItem build-evidence -Filter '*.result.json' | ForEach-Object { Get-Content $_ -Raw | ConvertFrom-Json } |
        Where-Object { $_.program -eq 'cargo' -and $_.arguments[0] -eq 'build' }
    $linkCommands = Get-Content (Join-Path build-evidence $buildResult.stdout_file)
    $finalLink = @($linkCommands | Where-Object { $_ -match '/OUT:.*texzi[-_]app.*\.exe' })
    if ($finalLink.Count -ne 1) { throw 'Cannot identify the actual final application linker invocation' }
    foreach ($selectedPath in @($effectiveLinker) + $selectedLibs) {
        if (!$finalLink[0].Contains($selectedPath.Replace('\', '\\'))) { throw "Actual link invocation missing selected path: $selectedPath" }
    }
    $finalLink | Set-Content build-evidence/final-app-link-command.txt
    Invoke-Checked cargo @('metadata', '--locked', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc') -StdoutFile 'build-evidence/metadata.json'
    New-Item dist/package/licenses -ItemType Directory -Force | Out-Null
    Copy-Item target/x86_64-pc-windows-msvc/release/texzi-app.exe dist/package/
    Copy-Item LICENSE,docs/tx00/RUN.md,docs/tx00/STARTUP_FAILURE.md dist/package/
    Copy-Item docs/tx00/licenses dist/package/licenses/supplemental -Recurse
    Invoke-Checked rustc @('--print', 'sysroot') -StdoutFile 'build-evidence/rust-sysroot.txt'
    $sysroot = (Get-Content build-evidence/rust-sysroot.txt -Raw).Trim()
    Copy-Item (Join-Path $sysroot 'share/doc/rust') dist/package/licenses/rust-runtime -Recurse
    $metadata = Get-Content build-evidence/metadata.json -Raw | ConvertFrom-Json
    $sbom = foreach ($package in $metadata.packages) {
        $folder = "dist/package/licenses/$($package.name)-$($package.version)"
        New-Item $folder -ItemType Directory -Force | Out-Null
        $root = Split-Path $package.manifest_path -Parent
        if ($package.license_file) {
            Copy-Item (Join-Path $root $package.license_file) -Destination $folder
        }
        Get-ChildItem $root -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)' } | Copy-Item -Destination $folder
        if ($package.name -eq 'mac') {
            Copy-Item (Join-Path $root 'README.md') $folder
            Copy-Item LICENSE (Join-Path $folder 'Apache-2.0.txt')
        }
        # NSH inherits the repository's root license.
        if ($package.source -like 'git+https://github.com/longbridge/nsh*') {
            $repoRoot = Split-Path (Split-Path $root -Parent) -Parent
            Get-ChildItem $repoRoot -File | Where-Object { $_.Name -match '^(LICENSE|NOTICE)' } | Copy-Item -Destination $folder -Force
        }
        [ordered]@{ name=$package.name; version=$package.version; source=$package.source; license=$package.license; license_file=$package.license_file; id=$package.id }
    }
    $sbom | ConvertTo-Json -Depth 6 | Set-Content dist/package/sbom.json
    Copy-Item build-evidence/metadata.json dist/package/
    "commit=$commit`nrun=$env:GITHUB_RUN_ID`nattempt=$env:GITHUB_RUN_ATTEMPT`nimage=$env:ImageOS/$env:ImageVersion`n" | Set-Content dist/package/build-identity.txt
    Get-FileHash dist/package/texzi-app.exe -Algorithm SHA256 | ForEach-Object { "$($_.Hash)  texzi-app.exe" } | Set-Content dist/package/SHA256SUMS.txt
} catch {
    $_ | Out-String | Add-Content build-evidence/build.log
    throw
} finally {
    Stop-Transcript
}
Copy-Item build-evidence/build.log dist/package/
Copy-Item build-evidence dist/package/build-evidence -Recurse
Compress-Archive dist/package/* dist/texzi-tx00-win-x64.zip
Get-FileHash dist/texzi-tx00-win-x64.zip -Algorithm SHA256 | ForEach-Object { "$($_.Hash)  texzi-tx00-win-x64.zip" } | Set-Content dist/SHA256SUMS.txt
Copy-Item build-evidence/build.log dist/build.log
