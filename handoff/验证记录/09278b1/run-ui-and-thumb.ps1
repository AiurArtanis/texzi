$ErrorActionPreference='Stop'
if(Get-Process cargo,rustc -ErrorAction SilentlyContinue){throw 'Wait for compilation before timing the desktop'}
$candidate=Get-Content "$PSScriptRoot/candidate-identity.json" -Raw|ConvertFrom-Json
$fixture=Get-Content 'D:/Work/texzi/evidence/v1-shell-820975f/novel-fixture.json' -Raw|ConvertFrom-Json
$run=Join-Path ([IO.Path]::GetTempPath()) ('texzi-ui-thumb-'+[guid]::NewGuid().ToString('N'))
$fixtures="$run-fixtures"
$null=New-Item -ItemType Directory -Path $fixtures
$novel=Join-Path $fixtures 'novel-thumb.txt'
Copy-Item -LiteralPath $fixture.file -Destination $novel
if((Get-FileHash -LiteralPath $novel).Hash -ne $fixture.sha256){throw 'Novel sample identity mismatch'}
$env:TEXZI_TRACE_RICH_ROWS='1'
. "$PSScriptRoot/window-command.ps1"
$result=[ordered]@{commit=$candidate.commit;exe_sha256=$candidate.exe_sha256;run=$run;fixture_sha256=$fixture.sha256}
$result|ConvertTo-Json|Set-Content "$PSScriptRoot/ui-thumb-current.json"
$mutex=[Threading.Mutex]::new($false,'Local\Texzi.Paladin.DesktopTest')
if(!$mutex.WaitOne(30000)){throw 'Paladin desktop already in use'}
try {
 $driver=Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @('-NoProfile','-File',"$PSScriptRoot/window-driver.ps1",'-Round',$run,'-Mode','trace','-Exe',$candidate.unpacked_exe,'-Commit',$candidate.commit,'-Hash',$candidate.exe_sha256) -WindowStyle Hidden -PassThru
 $deadline=[DateTime]::UtcNow.AddSeconds(30)
 while(!(Test-Path "$run/ready.json") -and !$driver.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 125}
 Invoke-TexziWindow @{action='resize';label='01-wide';x=25;y=25;w=1774;h=770}
 Invoke-TexziWindow @{action='title-metrics';label='02-metrics'}
 $m=Get-Content "$run/02-metrics.json" -Raw|ConvertFrom-Json
 $scale=$m.dpi/96.0
 $result.dpi=$m.dpi
 foreach($menu in @(@{key=70;label='file'},@{key=69;label='edit'},@{key=86;label='appearance'})){
  Invoke-TexziWindow @{action='keys';label="02-$($menu.label)-alt";keys=@(18,$menu.key);allow_menu_key=($menu.key -eq 86)}
  Invoke-TexziWindow @{action='title-menu';label="02-$($menu.label)-popup";optional=$true}
  Invoke-TexziWindow @{action='keys';label="02-$($menu.label)-escape";keys=@(27)}
 }
 Invoke-TexziWindow @{action='keys';label='03-ctrl-e';keys=@(17,69)}
 Invoke-TexziWindow @{action='keys';label='04-escape-file-search';keys=@(27)}
 Invoke-TexziWindow @{action='keys';label='05-f1';keys=@(112)}
 Invoke-TexziWindow @{action='text';label='06-light-command';text='texzi.appearance.light'}
 Invoke-TexziWindow @{action='keys';label='07-light';keys=@(13)}
 Invoke-TexziWindow @{action='keys';label='08-ctrl-shift-p';keys=@(17,16,80)}
 Invoke-TexziWindow @{action='text';label='09-dark-command';text='texzi.appearance.dark'}
 Invoke-TexziWindow @{action='keys';label='10-dark';keys=@(13)}
 Invoke-TexziWindow @{action='keys';label='10a-ctrl-p';keys=@(17,80)}
 Invoke-TexziWindow @{action='keys';label='10b-escape-file-search';keys=@(27)}
 Invoke-TexziWindow @{action='physical-open';label='11-open'}
 Invoke-TexziWindow @{action='wait-dialog';label='12-dialog'}
 Invoke-TexziWindow @{action='dialog-path';label='13-path';text=$novel}
 Invoke-TexziWindow @{action='wait-main';label='14-source'}
 $right=$m.origin.X+$m.width
 $y=[int]($m.origin.Y+99*$scale)
 Invoke-TexziWindow @{action='switch-samples';label='15-rich';x=[int]($right-35*$scale);y=$y;marks=@(90,250,700,1600,3500,5500,7200)}
 if((Get-Content "$run/stderr.log" -Raw) -notmatch 'mode_rich'){throw 'Rich mode was not reached'}
 $x=[int]($right-5*$scale)
 $sampleX=$x+2
 $top=[int]($m.origin.Y+113*$scale)
 $bottom=[int]($m.origin.Y+$m.height-20*$scale)
 $hints=@(477,452,329,279)+@(for($pos=$top+12;$pos -lt $bottom;$pos+=24){$pos})
 $detector=@{action='detect-thumb';x=$x;sampleX=$sampleX;top=$top;bottom=$bottom;guesses=@($hints|Where-Object {$_ -gt $top -and $_ -lt $bottom});minLength=[int](22*$scale)}
 function Drag($mode,[switch]$Reverse){
  $detector.label="$mode-geometry"
  Invoke-TexziWindow $detector
  $g=Get-Content "$run/$mode-geometry.json" -Raw|ConvertFrom-Json
  $distance=if($Reverse){-1*[Math]::Min(230,$g.y-$top-12)}else{[Math]::Min(238,$bottom-$g.y-8)}
  if([Math]::Abs($distance) -lt 95){throw 'Insufficient track for a long drag'}
  $positions=@(1..12|ForEach-Object {[int]($g.y+$distance*$_/12)})
  if($Reverse){
   $end=$positions[-1]
   $positions+=@(1..7|ForEach-Object {[int]($end-$distance*$_/9)})
  }
  Invoke-TexziWindow @{action='thumb-held';label=$mode;x=$g.x;y=$g.y;positions=$positions}
 }
 Drag '16-rich-long'
 Drag '17-rich-reverse' -Reverse
 Invoke-TexziWindow @{action='switch-samples';label='18-source';x=[int]($right-105*$scale);y=$y;marks=@(90,250,700,1450,2400)}
 if((Get-Content "$run/stderr.log" -Raw) -notmatch 'mode_source'){throw 'Source mode was not reached'}
 Drag '19-source-long'
 Drag '20-source-reverse' -Reverse
 Invoke-TexziWindow @{action='keys';label='21-find';keys=@(17,70)}
 Invoke-TexziWindow @{action='keys';label='22-close-find';keys=@(27)}
 Invoke-TexziWindow @{action='click';label='23-start-background-switch';x=[int]($right-35*$scale);y=$y}
 Invoke-TexziWindow @{action='keys';label='24-other-tab';keys=@(17,33)}
 Start-Sleep -Milliseconds 4500
 Invoke-TexziWindow @{action='capture';label='25-other-tab-after-completion'}
 Invoke-TexziWindow @{action='keys';label='26-return-novel';keys=@(17,34)}
 Invoke-TexziWindow @{action='close';label='27-close'}
 [void]$driver.WaitForExit(10000)
 $result.driverExit=$driver.ExitCode
 $result.app=Get-Content "$run/result.json" -Raw|ConvertFrom-Json
} finally {
 if($null -ne $driver -and !$driver.HasExited -and !(Test-Path "$run/result.json")){
  try{Invoke-TexziWindow @{action='abort';label='cleanup-owned-window'}}catch{Write-Warning $_.ToString()}
 }
 $mutex.ReleaseMutex()
 $mutex.Dispose()
 $result.fixture_after=(Get-FileHash -LiteralPath $novel).Hash
 $result|ConvertTo-Json -Depth 8|Tee-Object -FilePath "$PSScriptRoot/ui-thumb.json"
}
