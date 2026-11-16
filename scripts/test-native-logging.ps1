$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('texzi-logging-' + [guid]::NewGuid())
New-Item "$scratch/scripts", "$scratch/build-evidence" -ItemType Directory -Force | Out-Null
Copy-Item "$PSScriptRoot/native-command.ps1", "$PSScriptRoot/build-candidate.ps1" "$scratch/scripts/"
$originalDirectory = $PWD
try {
    Set-Location $scratch
    . "$scratch/scripts/native-command.ps1"
    $pwsh = Join-Path $PSHOME 'pwsh.exe'
    Invoke-Checked $pwsh @('-NoProfile', '-Command', '[Console]::Out.WriteLine("TX00_SUCCESS_STDOUT"); [Console]::Error.WriteLine("TX00_SUCCESS_STDERR"); exit 0')
    $rejected = $false
    try {
        Invoke-Checked $pwsh @('-NoProfile', '-Command', '[Console]::Out.WriteLine("TX00_FAILURE_STDOUT" + ("o" * 100000)); [Console]::Error.WriteLine("TX00_FAILURE_STDERR" + ("e" * 100000)); exit 23')
    } catch {
        if ($_.Exception.Message -notlike '*failed: 23') { throw }
        $rejected = $true
    }
    if (!$rejected) { throw 'Native failure was swallowed' }
    $results = @(Get-ChildItem build-evidence -Filter '*.result.json' | Sort-Object Name | ForEach-Object { Get-Content $_ -Raw | ConvertFrom-Json })
    if ($results.Count -ne 2 -or $results[0].exit_code -ne 0 -or $results[1].exit_code -ne 23) { throw 'Incorrect captured exit codes' }
    foreach ($index in 0..1) {
        $out = Get-Content (Join-Path build-evidence $results[$index].stdout_file) -Raw
        $err = Get-Content (Join-Path build-evidence $results[$index].stderr_file) -Raw
        if ($out -notmatch 'TX00_.*_STDOUT' -or $err -notmatch 'TX00_.*_STDERR') { throw 'Missing stream' }
    }
    if ((Get-Item build-evidence/command-002.stdout.log).Length -lt 100000 -or
        (Get-Item build-evidence/command-002.stderr.log).Length -lt 100000) { throw 'Large output truncated' }
    # A separate clean script root verifies the actual build entry point fails before packaging.
    $probe = Join-Path $scratch 'failure-probe'
    New-Item "$probe/scripts" -ItemType Directory -Force | Out-Null
    Copy-Item "$scratch/scripts/*" "$probe/scripts/"
    & $pwsh -NoProfile -File "$probe/scripts/build-candidate.ps1" -FailureProbe
    $entryExit = $LASTEXITCODE
    if ($entryExit -eq 0) { throw 'Build entry point swallowed native failure' }
    if (Test-Path "$probe/dist") { throw 'Failed build produced a candidate directory' }
    $failure = Get-Content "$probe/build-evidence/command-001.result.json" -Raw | ConvertFrom-Json
    $log = Get-Content "$probe/build-evidence/build.log" -Raw
    if ($failure.exit_code -ne 23 -or $log -notmatch 'TX00_FAILURE_STDOUT' -or
        $log -notmatch 'TX00_FAILURE_STDERR' -or $log -notmatch 'EXIT: 23') { throw 'Failure log incomplete' }
    Copy-Item "$probe/build-evidence" (Join-Path $originalDirectory 'logging-test-evidence') -Recurse
    "Logging contract verified: native exits 0/23, both streams, 100KB stream pressure, entry exit=$entryExit, no candidate. Scratch retained: $scratch"
} finally {
    Set-Location $originalDirectory
}
# The failure above is expected and asserted. Do not leak its LASTEXITCODE
# into GitHub's outer shell after this successful contract test.
exit 0
