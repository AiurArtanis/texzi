$ErrorActionPreference='Stop'
$i=Get-Content "$PSScriptRoot/candidate-identity.json"|ConvertFrom-Json
. "$PSScriptRoot/window-command.ps1"
Add-Type -Path "$PSScriptRoot/dpi.cs"
$interop=([regex]::Match((Get-Content "$PSScriptRoot/window-driver.ps1" -Raw),"(?s)Add-Type @'\r?\n(.+?)\r?\n'@").Groups[1].Value)
Add-Type -TypeDefinition $interop
Add-Type -AssemblyName System.Windows.Forms
[void][ImeOnlyProbe]::SetThreadDpiAwarenessContext([IntPtr](-4))
$mutex=[Threading.Mutex]::new($false,'Local\Texzi.Paladin.DesktopTest')
if(!$mutex.WaitOne(60000)){throw 'Desktop mutex timeout'}
$previous=[ImeOnlyProbe]::GetForegroundWindow()
$cursor=[Windows.Forms.Cursor]::Position
$baseline=[TexziDpi]::Read()
$baseline|ConvertTo-Json|Set-Content "$PSScriptRoot/dpi-before.json"
try {
 if($baseline.cur -ne 4 -or $baseline.min -ne 0){throw 'Unexpected baseline display scaling'}
 foreach($scale in 100,150){
  $offset=if($scale -eq 100){0}else{2}
  [TexziDpi]::Write($baseline,$offset)
  Start-Sleep -Milliseconds 750
  $run=Join-Path $PSScriptRoot "dpi-$scale"
  $driver=Start-Process (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @('-NoProfile','-File',"$PSScriptRoot/window-driver.ps1",'-Round',$run,'-Mode','trace','-Exe',$i.unpacked_exe,'-Commit',$i.commit,'-Hash',$i.exe_sha256) -WindowStyle Hidden -PassThru
  $deadline=[DateTime]::UtcNow.AddSeconds(30)
  while(!(Test-Path "$run/ready.json") -and !$driver.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 150}
  Invoke-TexziWindow @{action='resize';label='resize';x=25;y=25;w=1500;h=770}
  Invoke-TexziWindow @{action='title-metrics';label='metrics'}
  $m=Get-Content "$run/metrics.json"|ConvertFrom-Json
  if($m.dpi -ne $scale*96/100){throw "Expected $scale%, got $($m.dpi) DPI"}
  foreach($theme in 'dark','light'){
   Invoke-TexziWindow @{action='keys';label="$theme-theme-open";keys=@(112)}
   Invoke-TexziWindow @{action='text';label="$theme-theme-query";text="texzi.appearance.$theme"}
   Invoke-TexziWindow @{action='keys';label="$theme-theme-applied";keys=@(13)}
   foreach($menu in @(@{name='file';vk=70},@{name='edit';vk=69},@{name='appearance';vk=86})){
    $tag="$theme-$($menu.name)"
    Invoke-TexziWindow @{action='keys';label="$tag-open";keys=@(18,$menu.vk);allow_menu_key=($menu.vk -eq 86)}
    Invoke-TexziWindow @{action='title-menu';label="$tag-popup"}
    Invoke-TexziWindow @{action='keys';label="$tag-dismiss";keys=@(27)}
   }
   Invoke-TexziWindow @{action='keys';label="$theme-search-open";keys=@(17,16,80)}
   Invoke-TexziWindow @{action='text';label="$theme-search-replace";text='texzi.find.replaceOpen'}
   Invoke-TexziWindow @{action='keys';label="$theme-replace-panel";keys=@(13)}
   Invoke-TexziWindow @{action='keys';label="$theme-replace-dismiss";keys=@(27)}
   Invoke-TexziWindow @{action='keys';label="$theme-saveall-open";keys=@(112)}
   Invoke-TexziWindow @{action='text';label="$theme-saveall-query";text='texzi.file.saveAll'}
   Invoke-TexziWindow @{action='keys';label="$theme-saveall-dismiss";keys=@(27)}
  }
  Invoke-TexziWindow @{action='close';label='close'}
  [void]$driver.WaitForExit(10000)
  $driver.ExitCode | Set-Content "$run/driver.exit"
 }
}finally{
 try {
  if($driver -and !$driver.HasExited -and !(Test-Path "$run/result.json")){try{Invoke-TexziWindow @{action='abort';label='cleanup'}}catch{}}
 }finally{
  try {
   if([TexziDpi]::Read().cur -ne $baseline.cur){[TexziDpi]::Write($baseline,$baseline.cur)}
   [TexziDpi]::Read()|ConvertTo-Json|Set-Content "$PSScriptRoot/dpi-after.json"
   [Windows.Forms.Cursor]::Position=$cursor
   if($previous -ne [IntPtr]::Zero){[void][ImeOnlyProbe]::SetForegroundWindow($previous)}
  }finally{$mutex.ReleaseMutex();$mutex.Dispose()}
 }
}
