$ErrorActionPreference='Stop'
$candidate=Get-Content "$PSScriptRoot/candidate-identity.json" -Raw|ConvertFrom-Json
$run=Join-Path $PSScriptRoot "chord-round"
$fixtures="$run-fixtures"
$null=New-Item -ItemType Directory -Path $fixtures
$file=Join-Path $fixtures 'shortcut-save.txt'
[IO.File]::WriteAllText($file,'ORIGINAL')
$before=(Get-FileHash -LiteralPath $file).Hash
$env:TEXZI_TRACE_CHORD='1'
$mutex=[Threading.Mutex]::new($false,'Local\Texzi.Paladin.DesktopTest')
if(!$mutex.WaitOne(30000)){throw 'Paladin desktop already in use'}
. "$PSScriptRoot/window-command.ps1"
$result=[ordered]@{commit=$candidate.commit;exe_sha256=$candidate.exe_sha256;run=$run;before=$before}
try {
 $driver=Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @('-NoProfile','-File',"$PSScriptRoot/chord-driver.ps1",'-Round',$run,'-Mode','trace','-Exe',$candidate.unpacked_exe,'-Commit',$candidate.commit,'-Hash',$candidate.exe_sha256) -WindowStyle Hidden -PassThru
 $deadline=[DateTime]::UtcNow.AddSeconds(30)
 while(!(Test-Path "$run/ready.json") -and !$driver.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 125}
 Invoke-TexziWindow @{action='physical-open';label='01-open'}
 Invoke-TexziWindow @{action='wait-dialog';label='02-picker'}
 Invoke-TexziWindow @{action='dialog-path';label='03-file';text=$file}
 Invoke-TexziWindow @{action='wait-main';label='04-loaded'}
 Invoke-TexziWindow @{action='text';label='05-edit';text='SAVEDCHORD'}
 Invoke-TexziWindow @{action='keys';label='05a-test-latin-input';keys=@(160)}
 Invoke-TexziWindow @{action='save-chord-pair';label='06-chord-pair';fixture=$file;before=$before;pattern='continuation_hit.*texzi.file.saveAll'}
 $result.after_removed=(Get-Content "$run/06-chord-pair.json" -Raw|ConvertFrom-Json).after_removed
 $trace=@(Select-String -LiteralPath "$run/stderr.log" -Pattern 'TX_CHORD_SHELL .*stage=(prefix_begin|continuation_hit)')
 $events=@($trace|ForEach-Object {
   $m=[regex]::Match($_.Line,'utc_us=(\d+) stage=(\w+).* command=(\S+)$')
   if($m.Success){[pscustomobject]@{us=[long]$m.Groups[1].Value;stage=$m.Groups[2].Value;command=$m.Groups[3].Value}}
 })
 if(($events.stage -join ',') -ne 'prefix_begin,continuation_hit,prefix_begin,continuation_hit' -or
     $events[1].command -ne 'none' -or $events[3].command -ne 'texzi.file.saveAll') {throw 'Unexpected chord routes'}
 $result.second_prefix_gap_ms=($events[2].us-$events[0].us)/1000
 $result.save_since_first_prefix_ms=($events[3].us-$events[0].us)/1000
 $result.save_since_second_prefix_ms=($events[3].us-$events[2].us)/1000
 if($result.second_prefix_gap_ms -ge 1500 -or $result.save_since_first_prefix_ms -le 1500 -or $result.save_since_second_prefix_ms -ge 1500){throw 'Test did not straddle the old expiry'}
 $deadline=[DateTime]::UtcNow.AddSeconds(8)
 do{$body=[IO.File]::ReadAllText($file);if($body -match 'SAVEDCHORD'){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
 if($body -notmatch 'SAVEDCHORD' -or $body -notmatch 'ORIGINAL'){throw 'Save All did not persist the test edit'}
 $result.after_save=(Get-FileHash -LiteralPath $file).Hash
 $result.expected_original='ORIGINAL'
 $result.actual_saved_body=$body
 $result.input_mode='English mode selected by left Shift, then restored'
 Invoke-TexziWindow @{action='keys';label='10-ctrl-h';keys=@(17,72)}
 Invoke-TexziWindow @{action='keys';label='11-esc';keys=@(27)}
 Invoke-TexziWindow @{action='keys';label='11a-restore-input-mode';keys=@(160)}
 Invoke-TexziWindow @{action='close';label='12-close'}
 [void]$driver.WaitForExit(10000)
 $result.driverExit=$driver.ExitCode
 $result.app=Get-Content "$run/result.json" -Raw|ConvertFrom-Json
} finally {
 if($null -ne $driver -and !$driver.HasExited -and !(Test-Path "$run/result.json")){
  try{Invoke-TexziWindow @{action='abort';label='cleanup-owned-window'}}catch{Write-Warning $_.ToString()}
 }
 $mutex.ReleaseMutex()
 $mutex.Dispose()
 $result|ConvertTo-Json -Depth 6|Tee-Object "$PSScriptRoot/command-chords.json"
}

