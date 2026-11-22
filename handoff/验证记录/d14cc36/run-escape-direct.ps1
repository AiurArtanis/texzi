$ErrorActionPreference='Stop'
$run=Join-Path $PSScriptRoot 'escape-direct'
$i=Get-Content "$PSScriptRoot/candidate-identity.json"|ConvertFrom-Json
. "$PSScriptRoot/window-command.ps1"
$mutex=[Threading.Mutex]::new($false,'Local\Texzi.Paladin.DesktopTest')
if(!$mutex.WaitOne(90000)){throw 'Desktop mutex timeout'}
try{
 $driver=Start-Process (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @('-NoProfile','-File',"$PSScriptRoot/window-driver.ps1",'-Round',$run,'-Mode','trace','-Exe',$i.unpacked_exe,'-Commit',$i.commit,'-Hash',$i.exe_sha256) -WindowStyle Hidden -PassThru
 $deadline=[DateTime]::UtcNow.AddSeconds(30)
 while(!(Test-Path "$run/ready.json") -and !$driver.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 125}
 Invoke-TexziWindow @{action='resize';label='resize';x=25;y=25;w=1774;h=770}
 function OpenFile($name,$content){
   $file=Join-Path $run "fixtures/$name.txt"
   Set-Content $file $content
   Invoke-TexziWindow @{action='physical-open';label="$name-open"}
   Invoke-TexziWindow @{action='wait-dialog';label="$name-dialog"}
   Invoke-TexziWindow @{action='dialog-path';label="$name-path";text=$file}
   Invoke-TexziWindow @{action='wait-main';label="$name-visible"}
 }
 OpenFile 'control' 'STEAMYOLD control'
 OpenFile 'target' 'STEAMYOLD target'
 . "$PSScriptRoot/escape-direct-actions.ps1"
 Invoke-TexziWindow @{action='close';label='close'}
 [void]$driver.WaitForExit(10000)
 $driver.ExitCode | Set-Content "$run/driver.exit"
}finally{
 if($driver -and !$driver.HasExited -and !(Test-Path "$run/result.json")){try{Invoke-TexziWindow @{action='abort';label='cleanup'}}catch{}}
 $mutex.ReleaseMutex()
 $mutex.Dispose()
}


