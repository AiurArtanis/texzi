$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Set-Location (Split-Path $PSScriptRoot -Parent)
if (Test-Path dist) { throw 'dist already exists; use a clean checkout to avoid stale candidates.' }
New-Item build-evidence -ItemType Directory -Force | Out-Null
Start-Transcript -Path build-evidence/build.log
try {
    function Invoke-Checked([string]$Program, [string[]]$Arguments) {
        Write-Host ('COMMAND: ' + $Program + ' ' + ($Arguments -join ' '))
        & $Program @Arguments
        $code = $LASTEXITCODE
        Write-Host "EXIT: $code"
        if ($code -ne 0) { throw "$Program failed: $code" }
    }
    $commit = (git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify commit' }
    if ($env:GITHUB_SHA -and $commit -ne $env:GITHUB_SHA) { throw 'Checkout SHA mismatch' }
    Get-ComputerInfo | Select-Object WindowsProductName, WindowsVersion, OsBuildNumber, OsArchitecture | Format-List
    Write-Host "commit=$commit run=$env:GITHUB_RUN_ID attempt=$env:GITHUB_RUN_ATTEMPT image=$env:ImageOS/$env:ImageVersion"
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    Invoke-Checked $vswhere @('-latest', '-products', '*', '-format', 'json')
    Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\Lib" | Select-Object Name
    $vs = & $vswhere -latest -products '*' -property installationPath
    Get-Content (Join-Path $vs 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt')
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
    & cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc > build-evidence/metadata.json
    if ($LASTEXITCODE -ne 0) { throw 'metadata failed' }
    New-Item dist/package/licenses -ItemType Directory -Force | Out-Null
    Copy-Item target/x86_64-pc-windows-msvc/release/texzi-app.exe dist/package/
    Copy-Item LICENSE,docs/tx00/RUN.md dist/package/
    $metadata = Get-Content build-evidence/metadata.json -Raw | ConvertFrom-Json
    $sbom = foreach ($package in $metadata.packages) {
        $folder = "dist/package/licenses/$($package.name)-$($package.version)"
        New-Item $folder -ItemType Directory -Force | Out-Null
        $root = Split-Path $package.manifest_path -Parent
        if ($package.license_file) {
            Copy-Item (Join-Path $root $package.license_file) -Destination $folder
        }
        Get-ChildItem $root -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)' } | Copy-Item -Destination $folder
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
} finally {
    Stop-Transcript
}
Copy-Item build-evidence/build.log dist/package/
Compress-Archive dist/package/* dist/texzi-tx00-win-x64.zip
Get-FileHash dist/texzi-tx00-win-x64.zip -Algorithm SHA256 | ForEach-Object { "$($_.Hash)  texzi-tx00-win-x64.zip" } | Set-Content dist/SHA256SUMS.txt
Copy-Item build-evidence/build.log dist/build.log
