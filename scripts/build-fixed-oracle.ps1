# Runs only the reviewed independent test against a byte-verified fixed export.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$product='5a14645d38dbab6e6612cea5e0a585c740fb6cd7'
$expectedHarness='60D4A5F5BBA71F359488A1F9D27F9A0ECA25E4DB7A0F79F49C17A6D12ECD53F0'
$harness=Join-Path $repo 'tests/oracles/5a14645/steamy_oracle.rs'
$artifact=Join-Path $repo 'oracle-artifact'
if(Test-Path -LiteralPath $artifact){throw 'Preserve existing oracle-artifact; use a clean checkout'}
New-Item -ItemType Directory -Path $artifact | Out-Null
$tempRoot=if($env:RUNNER_TEMP){$env:RUNNER_TEMP}else{[IO.Path]::GetTempPath()}
$sandbox=Join-Path $tempRoot ('texzi-fixed-oracle-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $sandbox | Out-Null
$source=Join-Path $sandbox 'fixed-source'
$identity=[ordered]@{schema=1;product_commit=$product;harness_sha256=$expectedHarness;orchestration_commit=$env:GITHUB_SHA;run=$env:GITHUB_RUN_ID;attempt=$env:GITHUB_RUN_ATTEMPT;image_os=$env:ImageOS;image_version=$env:ImageVersion;status='preparing';test_executable_sha256=$null}
Set-Location $sandbox
New-Item -ItemType Directory build-evidence | Out-Null
. (Join-Path $PSScriptRoot 'native-command.ps1')
try {
    Invoke-Checked git @('-C',$repo,'rev-parse','HEAD') -StdoutFile 'build-evidence/orchestration-commit.txt'
    $head=(Get-Content build-evidence/orchestration-commit.txt -Raw).Trim()
    if($env:GITHUB_SHA -and $head -ne $env:GITHUB_SHA){throw 'Orchestration checkout mismatch'}
    $identity.orchestration_commit=$head
    if((Get-FileHash -LiteralPath $harness -Algorithm SHA256).Hash -ne $expectedHarness){throw 'Reviewed harness changed'}
    Invoke-Checked git @('-C',$repo,'-c','core.autocrlf=false','-c','core.eol=lf','archive','--format=zip','--output',"$artifact/fixed-product-source.zip",$product)
    Expand-Archive -LiteralPath "$artifact/fixed-product-source.zip" -DestinationPath $source
    $injected=Join-Path $source 'crates/texzi-document/tests/steamy_oracle.rs'
    if(Test-Path -LiteralPath $injected){throw 'Fixed product already has injected target'}
    Copy-Item -LiteralPath $harness -Destination $injected
    Invoke-Checked python @((Join-Path $PSScriptRoot 'verify-fixed-oracle-source.py'),'--repo',$repo,'--source',$source,'--out',"$artifact/source-manifest.json")
    Copy-Item -LiteralPath $harness -Destination "$artifact/steamy_oracle.rs"
    Copy-Item "$source/Cargo.lock","$source/rust-toolchain.toml","$source/Cargo.toml","$source/LICENSE" $artifact
    New-Item -ItemType Directory "$artifact/orchestration" | Out-Null
    Copy-Item "$PSScriptRoot/build-fixed-oracle.ps1","$PSScriptRoot/verify-fixed-oracle-source.py","$PSScriptRoot/native-command.ps1","$PSScriptRoot/select-linker.ps1" "$artifact/orchestration/"
    Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,OSArchitecture | ConvertTo-Json | Set-Content "$artifact/runner-os.json"
    $pin=Get-Content "$source/rust-toolchain.toml" -Raw
    if($pin -notmatch '(?m)^channel\s*=\s*"(1\.97\.0)"\s*$'){throw 'Unexpected fixed product toolchain'}
    $env:RUSTUP_TOOLCHAIN=$Matches[1]
    $vswhere="${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    Invoke-Checked $vswhere @('-latest','-products','*','-property','installationPath') -StdoutFile 'build-evidence/vs-installation.txt'
    $vs=(Get-Content build-evidence/vs-installation.txt -Raw).Trim()
    $defaultMsvc=Get-Content (Join-Path $vs 'VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt')
    . (Join-Path $PSScriptRoot 'select-linker.ps1')
    Invoke-Checked rustc @('-Vv')
    Invoke-Checked cargo @('-V')
    Invoke-Checked python @('--version')
    $manifest=Join-Path $source 'Cargo.toml'
    $config=Join-Path $source '.cargo/config.toml'
    Invoke-Checked cargo @('--config',$config,'tree','--manifest-path',$manifest,'-p','texzi-document','--locked','--target','x86_64-pc-windows-msvc','-e','features')
    Invoke-Checked cargo @('--config',$config,'test','--manifest-path',$manifest,'-p','texzi-document','--test','steamy_oracle','--release','--locked','--target','x86_64-pc-windows-msvc','--target-dir',"$sandbox/target",'--no-run','--message-format=json') -StdoutFile 'build-evidence/oracle-build.jsonl'
    $records=Get-Content build-evidence/oracle-build.jsonl | Where-Object { $_.StartsWith('{') } | ForEach-Object { $_ | ConvertFrom-Json }
    $executables=@($records | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'steamy_oracle' -and $_.executable } | Select-Object -ExpandProperty executable)
    if($executables.Count -ne 1){throw 'Expected exactly one compiled steamy_oracle executable'}
    Copy-Item -LiteralPath $executables[0] -Destination "$artifact/steamy_oracle.exe"
    $identity.test_executable_sha256=(Get-FileHash "$artifact/steamy_oracle.exe" -Algorithm SHA256).Hash
    $links=@(Get-Content build-evidence/oracle-build.jsonl | Where-Object { $_ -match '/OUT:.*steamy_oracle.*\.exe' })
    if($links.Count -ne 1){throw 'Missing unique actual oracle link command'}
    foreach($path in @($effectiveLinker)+$selectedLibs){if(!$links[0].Contains($path.Replace('\','\\'))){throw "Actual oracle link missing selected path: $path"}}
    $links | Set-Content build-evidence/final-oracle-link-command.txt
    Invoke-Checked python @((Join-Path $PSScriptRoot 'verify-fixed-oracle-source.py'),'--repo',$repo,'--source',$source,'--out',"$artifact/source-manifest-after-build.json")
    $identity.status='built'
    # Copy before execution: even an assertion failure must preserve the real exe.
    Invoke-Checked "$artifact/steamy_oracle.exe" @('--nocapture','--test-threads=1')
    $identity.status='executed-successfully'
} catch {
    $identity.status='failed'
    $identity['error']=$_.ToString()
    $_ | Out-String | Add-Content build-evidence/build.log
    throw
} finally {
    $identity | ConvertTo-Json -Depth 6 | Set-Content "$artifact/identity.json"
    if(Test-Path build-evidence){Copy-Item build-evidence "$artifact/build-evidence" -Recurse}
    $hashes=Get-ChildItem -LiteralPath $artifact -File -Recurse | ForEach-Object { [ordered]@{path=[IO.Path]::GetRelativePath($artifact,$_.FullName);bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash} }
    @($hashes) | ConvertTo-Json -Depth 5 | Set-Content "$artifact/artifact-files.sha256.json"
}
