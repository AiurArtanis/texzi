$ErrorActionPreference='Stop'
$run=Join-Path $PSScriptRoot 'explorer-final'
$i=Get-Content "$PSScriptRoot/candidate-identity.json"|ConvertFrom-Json
. "$PSScriptRoot/window-command.ps1"
$interop=([regex]::Match((Get-Content "$PSScriptRoot/explorer-driver.ps1" -Raw),"(?s)Add-Type @'\r?\n(.+?)\r?\n'@").Groups[1].Value)
Add-Type -TypeDefinition $interop
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Windows.Forms
[void][ImeOnlyProbe]::SetThreadDpiAwarenessContext([IntPtr](-4))
$mutex=[Threading.Mutex]::new($false,'Local\Texzi.Paladin.DesktopTest')
if(!$mutex.WaitOne(30000)){throw 'Desktop mutex timeout'}
$cursor=[Windows.Forms.Cursor]::Position
$previous=[ImeOnlyProbe]::GetForegroundWindow()
try{
 $driver=Start-Process (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @('-NoProfile','-File',"$PSScriptRoot/explorer-driver.ps1",'-Round',$run,'-Mode','trace','-Exe',$i.unpacked_exe,'-Commit',$i.commit,'-Hash',$i.exe_sha256) -WindowStyle Hidden -PassThru
 $deadline=[DateTime]::UtcNow.AddSeconds(30)
 while(!(Test-Path "$run/ready.json") -and !$driver.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 125}
 Invoke-TexziWindow @{action='resize';label='resize';x=1030;y=25;w=1470;h=850}
 $test=Get-Content "$run/ready.json"|ConvertFrom-Json
 $target=Join-Path $run 'fixtures/original.txt'
 Set-Content $target 'ORIGINALSTEAMYCONTENT'
 Invoke-TexziWindow @{action='physical-open';label='open-original'}
 Invoke-TexziWindow @{action='wait-dialog';label='dialog-original'}
 Invoke-TexziWindow @{action='dialog-path';label='path-original';text=$target}
 Invoke-TexziWindow @{action='wait-main';label='before-drop'}
 $folder=(New-Item -ItemType Directory "$run/drop-files").FullName
 Set-Content "$folder/A-success.txt" 'SUCCESSSTEAMYCONTENT'
 Set-Content "$folder/B-blocked.txt" 'BLOCKEDSTEAMYCONTENT'
 [IO.File]::WriteAllBytes("$folder/B-blocked.txt",[byte[]](0xEF,0xBB,0xBF,0xFF,0xFE))
 $shell=New-Object -ComObject Shell.Application
 $existing=@($shell.Windows()|ForEach-Object HWND)
 Start-Process explorer.exe -ArgumentList '/n,',$folder
 $deadline=[DateTime]::UtcNow.AddSeconds(14)
 do {
   $matches=@($shell.Windows()|Where-Object { $_.HWND -notin $existing -and $_.Document.Folder.Self.Path -eq $folder })
   if($matches.Count -eq 1){$explorer=$matches[0];break}
   Start-Sleep -Milliseconds 200
 }while([DateTime]::UtcNow -lt $deadline)
 if(!$explorer){throw 'No dedicated Explorer window for fixture folder'}
 $eh=[IntPtr]$explorer.HWND
 $ep=[ImeOnlyProbe]::Owner($eh)
 if((Get-Process -Id $ep).ProcessName -ne 'explorer'){throw 'Unexpected source process'}
 [void][ImeOnlyProbe]::MoveWindow($eh,25,35,950,760,$true)
 Start-Sleep -Milliseconds 500
 $doc=$explorer.Document
 $good=$doc.Folder.ParseName('A-success.txt')
 $bad=$doc.Folder.ParseName('B-blocked.txt')
 $doc.SelectItem($good,29)
 $doc.SelectItem($bad,1)
 $selected=@($doc.SelectedItems()|ForEach-Object Path|Sort-Object)
 if($selected.Count -ne 2 -or $selected[0] -ne "$folder\A-success.txt" -or $selected[1] -ne "$folder\B-blocked.txt"){throw "Unexpected selection $selected"}
 $root=[Windows.Automation.AutomationElement]::FromHandle($eh)
 $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::ListItem)
 $item=@($root.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)|Where-Object {$_.Current.Name -match '^A-success(\.txt)?$'})
 if($item.Count -ne 1){throw 'Explorer good item UIA node missing'}
 $rect=$item[0].Current.BoundingRectangle
 $x=[int]($rect.Left+[Math]::Min(18,$rect.Width/2))
 $y=[int]($rect.Top+$rect.Height/2)
 [void][ImeOnlyProbe]::SetForegroundWindow($eh)
 if([ImeOnlyProbe]::GetForegroundWindow() -ne $eh){
  $front=[ImeOnlyProbe+Point]::new();$front.X=700;$front.Y=60
  if([ImeOnlyProbe]::Owner([ImeOnlyProbe]::WindowFromPoint($front)) -ne $ep){throw 'Explorer activation is obscured'}
  [ImeOnlyProbe]::Click($front.X,$front.Y)
 }
 Start-Sleep -Milliseconds 220
 if([ImeOnlyProbe]::GetForegroundWindow() -ne $eh){throw 'Explorer not foreground'}
 Add-Type -AssemblyName System.Drawing
 $bounds=[ImeOnlyProbe+Rect]::new()
 [void][ImeOnlyProbe]::GetWindowRect($eh,[ref]$bounds)
 $bmp=[Drawing.Bitmap]::new($bounds.R-$bounds.L,$bounds.B-$bounds.T)
 $g=[Drawing.Graphics]::FromImage($bmp)
 try{$g.CopyFromScreen($bounds.L,$bounds.T,0,0,$bmp.Size);$bmp.Save("$run/explorer-before.png")}finally{$g.Dispose();$bmp.Dispose()}

 @{explorer_hWnd=$eh.ToInt64();explorer_pid=$ep;selection=$selected;source_x=$x;source_y=$y;target_x=2000;target_y=450;failed_item='B-blocked.txt invalid UTF-8 with explicit UTF-8 BOM'}|ConvertTo-Json|Set-Content "$run/explorer-source.json"
 Invoke-TexziWindow @{action='drop-ole';label='mixed-drop';sourceHwnd=$eh.ToInt64();sourcePid=$ep;x=$x;y=$y;nx=2000;ny=450}
 Invoke-TexziWindow @{action='keys';label='after-warning';keys=@(27)}
 Invoke-TexziWindow @{action='click';label='original-after';x=1850;y=155}

 Get-FileHash $target | ConvertTo-Json | Set-Content "$run/original-after.json"
 Invoke-TexziWindow @{action='close';label='close'}
 [void]$driver.WaitForExit(10000)
 $driver.ExitCode | Set-Content "$run/driver.exit"
}finally{
 if($lock){$lock.Dispose()}
 if($explorer){$explorer.Quit()}
 if($driver -and !$driver.HasExited -and !(Test-Path "$run/result.json")){try{Invoke-TexziWindow @{action='abort';label='cleanup'}}catch{}}
 [Windows.Forms.Cursor]::Position=$cursor
 if($previous -ne [IntPtr]::Zero){[void][ImeOnlyProbe]::SetForegroundWindow($previous)}
 $mutex.ReleaseMutex()
 $mutex.Dispose()
}


