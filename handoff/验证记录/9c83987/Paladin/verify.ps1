param([string]$IdentityPath, [switch]$Baseline)
$ErrorActionPreference='Stop'
$candidate=Get-Content -LiteralPath $IdentityPath -Raw | ConvertFrom-Json
$driverPath='D:/Work/texzi/evidence/v0.2-edit-menu'
. "$driverPath/window-command.ps1"
$case=if($Baseline){'baseline'}else{$candidate.commit.Substring(0,7)}
$run=Join-Path $PSScriptRoot "$case-$([DateTime]::Now.ToString('HHmmss'))"
$guard=[Threading.Mutex]::new($false,'Local\Texzi.Paladin.DesktopTest')
$driver=$null
$held=$false
$checks=[ordered]@{commit=$candidate.commit;exe_sha256=$candidate.exe_sha256;run=$run}
function Run($Label,$Action,$Extra=@{}) {
    Invoke-TexziWindow (@{action=$Action;label=$Label}+$Extra)
}
function Keys($Label,[int[]]$Codes) {
    Run $Label 'keys' @{keys=$Codes;allow_menu_key=(($Codes -join ',') -eq '18,86')}
}
function Click($Label,$X,$Y) {
    Run $Label 'click' @{x=[int]($layout.origin.X+$scale*$X);y=[int]($layout.origin.Y+$scale*$Y)}
}
function Open-File($Label,$Path) {
    Run "$Label-open" 'physical-open'
    Run "$Label-dialog" 'wait-dialog'
    Run "$Label-path" 'dialog-path' @{text=$Path}
    Run "$Label-main" 'wait-main'
}
function Button-Bounds([string[]]$Names) {
    Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
    $root=[Windows.Automation.AutomationElement]::FromHandle([IntPtr][long]$layout.hwnd)
    if($root.Current.ProcessId -ne $layout.pid){throw 'Unexpected automation root'}
    foreach($name in $Names){
        $match=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty,$name)
        foreach($element in $root.FindAll([Windows.Automation.TreeScope]::Descendants,$match)){
            $value=$element.Current
            $rect=$value.BoundingRectangle
            if($value.ProcessId -eq $layout.pid -and $rect.Width -gt 0 -and $rect.Height -gt 0 -and !$value.IsOffscreen){
                [pscustomobject]@{name=$name;x=($rect.Left-$layout.origin.X)/$scale;y=($rect.Top-$layout.origin.Y)/$scale;width=$rect.Width/$scale;height=$rect.Height/$scale}
            }
        }
    }
}
function Read-Shots([string]$Pattern) {
    $env:TEXZI_OCR_DIR=$run
    $env:TEXZI_OCR_PATTERN=$Pattern
    $env:TEXZI_OCR_LANG='en-US'
    $env:TEXZI_OCR_SCRIPT="$PSScriptRoot/ocr-anchor.ps1"
    $out=& "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" -NoProfile -NonInteractive -Command '& ([scriptblock]::Create([IO.File]::ReadAllText($env:TEXZI_OCR_SCRIPT))) -Directory $env:TEXZI_OCR_DIR'
    if($LASTEXITCODE -ne 0){throw 'Could not read screenshots'}
    @($out | Out-String | ConvertFrom-Json)
}
try {
    try{$held=$guard.WaitOne(30000)}catch [Threading.AbandonedMutexException]{$held=$true}
    if(!$held){throw 'Paladin desktop already in use'}
    $driver=Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @(
        '-NoProfile','-File',"$driverPath/window-driver.ps1",'-Round',$run,'-Mode','trace',
        '-Exe',$candidate.unpacked_exe,'-Commit',$candidate.commit,'-Hash',$candidate.exe_sha256
    ) -WindowStyle Hidden -PassThru
    $deadline=[DateTime]::UtcNow.AddSeconds(30)
    while(!(Test-Path "$run/ready.json") -and !$driver.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 125}
    Run '00-size' 'resize' @{x=25;y=25;w=1774;h=770}
    Run '00-metrics' 'title-metrics'
    $layout=Get-Content -LiteralPath "$run/00-metrics.json" -Raw | ConvertFrom-Json
    $scale=$layout.dpi/96
    $checks.layout=$layout
    if($layout.dpi -ne 192 -or $layout.width -ne 1748 -or $layout.height -ne 755){throw 'Unexpected window geometry'}
    $folder=Join-Path $run 'fixtures'
    $txt=Join-Path $folder 'report.txt'
    $md=Join-Path $folder 'notes.md'
    $textContent="第一行 Report"+[Environment]::NewLine+"第二行 draft"
    $markdownContent="# 原生 Markdown"+[Environment]::NewLine+[Environment]::NewLine+"展开 **粗体** 与正文。"
    $markdownContent = (1..60 | ForEach-Object {"## Section $_`r`n`r`nAnchor-$_ **bold** tail.`r`n"}) -join "`r`n"
    [IO.File]::WriteAllText($txt,$textContent)
    [IO.File]::WriteAllText($md,$markdownContent)
    Open-File '03-markdown' $md
    Click '03-focus-body' 480 245
    Run '03-wheel' 'wheel' @{x=[int]($layout.origin.X+780*$scale);y=[int]($layout.origin.Y+225*$scale);delta=-2400}
    Start-Sleep -Milliseconds 900
    Run '03-find-anchor' 'capture'
    $probe=Read-Shots '^03-find-anchor\.png$'
    $line=$probe[0].boxes | Where-Object {$_.text -match 'Anchor\s*-\s*[1Il]\s*[1Il]'} | Select-Object -First 1
    if(!$line){throw 'Section 11 paragraph is missing before the first switch'}
    $frame=Get-Content -LiteralPath "$run/trace.jsonl" | ForEach-Object {ConvertFrom-Json $_} |
        Where-Object {$_.kind -eq 'screenshot' -and $_.label -eq '03-find-anchor'} | Select-Object -Last 1
    $y=($frame.rect.T+($line.top+$line.bottom)/2-$layout.origin.Y)/$scale
    Click '03-place-caret' 490 $y
    Run '03-markdown-rich' 'capture'
    if(@(Button-Bounds @('行 43，列 25')).Count -ne 1 -or @(Button-Bounds @('未保存')).Count){throw 'Unexpected start caret or dirty state'}
    if(!$Baseline){
        $rich=@(Button-Bounds @('切换到源码'))
        if($rich.Count -ne 1 -or $rich[0].width -ne 32 -or $rich[0].height -ne 28){throw 'Missing 32 x 28 DIP Markdown mode target'}
        $checks.markdown_rich_dip=$rich[0]
        $modeX=$rich[0].x+16
        $modeY=$rich[0].y+14
        Click '03-switch-source' $modeX $modeY
        Start-Sleep -Milliseconds 650
        Run '03-markdown-source' 'capture'
        if(@(Button-Bounds @('行 43，列 25')).Count -ne 1 -or @(Button-Bounds @('未保存')).Count){throw 'Unexpected source caret or dirty state'}
        $source=@(Button-Bounds @('返回所见即所得'))
        if($source.Count -ne 1 -or $source[0].x -ne $rich[0].x -or $source[0].y -ne $rich[0].y){throw 'Markdown button shifted between modes'}
        $checks.markdown_source_dip=$source[0]
        Click '03-switch-rich' $modeX $modeY
        Start-Sleep -Milliseconds 650
        Run '03-markdown-restored' 'capture'
        if(@(Button-Bounds @('切换到源码')).Count -ne 1){throw 'Mode button did not return to rich text'}
    }else{
        Click '03-switch-source' ($layout.width/$scale-113) 101
        Start-Sleep -Milliseconds 650
        Run '03-markdown-source' 'capture'
        if(@(Button-Bounds @('行 43，列 25')).Count -ne 1 -or @(Button-Bounds @('未保存')).Count){throw 'Unexpected source caret or dirty state'}
    }
    if([IO.File]::ReadAllText($md) -cne $markdownContent){throw 'Mode switch changed the Markdown file'}
    $checks.markdown_file_unchanged=$true
    $frames=Read-Shots '^03-markdown-(rich|source|restored)\.png$'
    $checks.frames=@(foreach($frame in $frames){
        [pscustomobject]@{file=$frame.file;paragraph=($frame.text -match 'Anchor\s*-\s*[1Il]\s*[1Il]');heading=($frame.text -match 'Section\s+[1Il]\s*[1Il]')}
    })
    if($checks.frames.Count -ne 3 -or ($checks.frames | Where-Object {!$_.paragraph -or !$_.heading})){
        throw 'Section 11 or its paragraph left the viewport'
    }
    $expectedAnchor='行 43，列 25'
    if(@(Button-Bounds @($expectedAnchor)).Count -ne 1){throw 'Unexpected caret after round trip'}
    if(@(Button-Bounds @('未保存')).Count){throw 'A clean file became dirty after switching'}
    Run '04-type-without-click' 'text' @{text='STEAMYEND'}
    if(@(Button-Bounds @('未保存')).Count -ne 1){throw 'Typing did not update the active document'}
    Click '04-dirty-switch-source' $modeX $modeY
    Start-Sleep -Milliseconds 650
    if(@(Button-Bounds @('未保存')).Count -ne 1){throw 'Mode switch hid the dirty state'}
    Keys '04-save' @(17,83)
    $expected=$markdownContent.Replace('Anchor-11 **bold** tail.','Anchor-11 **bold** tail.STEAMYEND')
    if([IO.File]::ReadAllText($md) -cne $expected){throw 'Typing did not persist at the original caret'}
    $checks.round_trip_caret=$expectedAnchor
    $checks.direct_input_saved=$true
    $checks.dirty_survived_switch=$true
    Click '05-return-rich' $modeX $modeY
    Start-Sleep -Milliseconds 650
    if(@(Button-Bounds @('段落样式','插入或编辑链接','退出代码块')).Count){throw 'Formatting command occupying the default editor layout'}
    $format=@(Button-Bounds @('格式命令'))
    if($format.Count -ne 1){throw 'Missing formatting menu trigger'}
    Click '05-format-open' ($format[0].x+16) ($format[0].y+14)
    Run '05-format-popup' 'title-menu'
    Run '05-format-paragraph' 'click-popup-entry' @{item='段落样式'}
    Run '05-format-dialog' 'capture'
    if(!@(Button-Bounds @('段落样式')).Count){throw 'Paragraph format dialog did not open'}
    Keys '05-format-dismiss' @(27)
    $checks.format_popup_and_dialog=$true
    Keys '06-format-palette' @(112)
    Run '06-format-query' 'text' @{text='texzi.markdown.insertLink'}
    Keys '06-open-link' @(13)
    Run '06-link-dialog' 'capture'
    if(!@(Button-Bounds @('插入链接','链接属性')).Count){throw 'F1 did not open the link form'}
    Keys '06-close-link' @(27)
    $checks.format_palette_and_dialog=$true
    Run '99-close' 'close'
    if(!$driver.WaitForExit(10000)){throw 'Driver did not exit'}
    $checks.driver_exit=$driver.ExitCode
    $checks.application=Get-Content -LiteralPath "$run/result.json" -Raw | ConvertFrom-Json
    if($checks.driver_exit -ne 0 -or $checks.application.exit -ne 0){throw 'Driver or application exit failed'}
    $checks.mode_trace=@(Select-String -LiteralPath "$run/stdout.log","$run/stderr.log" -Pattern 'TEXZI_MODE_VIEW' | ForEach-Object Line)
    if($checks.mode_trace.Count -lt 4){throw 'Missing before/after view traces'}
    $checks.first_round_trip_caret_top_dip=@(foreach($entry in $checks.mode_trace[0..3]){
        if($entry -notmatch '行 43，列 25.*caret_top=Some\(([0-9.]+)\)'){throw "Missing visible caret geometry: $entry"}
        [double]::Parse($Matches[1],[Globalization.CultureInfo]::InvariantCulture)
    })
}finally{
    if($null -ne $driver -and !$driver.HasExited -and !(Test-Path "$run/result.json")){
        try{Run 'cleanup-owned-window' 'abort'}catch{Write-Warning $_.ToString()}
    }
    if($held){$guard.ReleaseMutex()}
    $guard.Dispose()
    if(Test-Path $run){$checks | ConvertTo-Json -Depth 7 | Tee-Object -FilePath "$run/summary.json"}
}



