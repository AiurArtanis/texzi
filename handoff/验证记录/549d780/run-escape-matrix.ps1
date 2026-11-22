param(
    [Parameter(Mandatory)][string]$IdentityPath,
    [switch]$Baseline
)
$ErrorActionPreference='Stop'
$candidate=Get-Content -Raw -LiteralPath $IdentityPath | ConvertFrom-Json
$previous=$PSScriptRoot
. "$previous/window-command.ps1"
$run=Join-Path $PSScriptRoot "matrix-round"
$fixture="$run-test.txt"
[IO.File]::WriteAllText($fixture,'ESCAPEBASE')
$mutex=[Threading.Mutex]::new($false,'Local\Texzi.Paladin.DesktopTest')
if(!$mutex.WaitOne(30000)){throw 'Paladin desktop already in use'}
$result=[ordered]@{commit=$candidate.commit;exe_sha256=$candidate.exe_sha256;run=$run;fixture=$fixture}
$rows=@()
try {
    $driver=Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @(
        '-NoProfile','-File',"$previous/window-driver.ps1",'-Round',$run,'-Mode','trace',
        '-Exe',$candidate.unpacked_exe,'-Commit',$candidate.commit,'-Hash',$candidate.exe_sha256
    ) -WindowStyle Hidden -PassThru
    $deadline=[DateTime]::UtcNow.AddSeconds(30)
    while(!(Test-Path "$run/ready.json") -and !$driver.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 125}
    Invoke-TexziWindow @{action='resize';label='00-size';x=25;y=25;w=1774;h=770}
    Invoke-TexziWindow @{action='title-metrics';label='00-metrics'}
    $layout=Get-Content -Raw -LiteralPath "$run/00-metrics.json" | ConvertFrom-Json
    if($layout.dpi -ne 192 -or $layout.width -lt 1730){throw 'Expected the observed 200% layout for test coordinates'}
    $result.dpi=$layout.dpi
    if(!$Baseline){
        Invoke-TexziWindow @{action='physical-open';label='00-open'}
        Invoke-TexziWindow @{action='wait-dialog';label='00-picker'}
        Invoke-TexziWindow @{action='dialog-path';label='00-file';text=$fixture}
        Invoke-TexziWindow @{action='wait-main';label='00-loaded'}
    }
    [string[]]$matrix=if($Baseline){'hotkey-default'}else{@(
        'menu-default','menu-find','palette-default','palette-find','hotkey-default','hotkey-find','menu-editor'
    )}
    for($i=0;$i -lt $matrix.Count;$i++){
        $entry,$focus=$matrix[$i] -split '-'
        $name='{0:00}-{1}-{2}' -f ($i+1),$entry,$focus
        $focusProbe=$null
        switch($entry){
            'menu' {
                Invoke-TexziWindow @{action='keys';label="$name-menu";keys=@(18,69)}
                Invoke-TexziWindow @{action='click-popup-entry';label="$name-open";item='替换'}
            }
            'palette' {
                Invoke-TexziWindow @{action='keys';label="$name-f1";keys=@(112)}
                Invoke-TexziWindow @{action='text';label="$name-command";text='texzi.find.replaceOpen'}
                Invoke-TexziWindow @{action='keys';label="$name-open";keys=@(13)}
            }
            'hotkey' {Invoke-TexziWindow @{action='keys';label="$name-open";keys=@(17,72)}}
        }
        if($focus -eq 'find'){
            $focusProbe=@{menu='PROBEALPHA';palette='PROBEBETA';hotkey='PROBEGAMMA'}[$entry]
            Invoke-TexziWindow @{action='click';label="$name-click-find";x=($layout.origin.X+610);y=($layout.origin.Y+518)}
            Invoke-TexziWindow @{action='text';label="$name-before";text=$focusProbe}
        } elseif($focus -eq 'editor'){
            Invoke-TexziWindow @{action='click';label="$name-before";x=($layout.origin.X+750);y=($layout.origin.Y+275)}
        } else {
            Copy-Item -LiteralPath "$run/$name-open.png" -Destination "$run/$name-before.png"
        }
        Invoke-TexziWindow @{action='keys';label="$name-after";keys=@(27)}
        $editable=$null
        if(!$Baseline){
            $probe="EDITOK$($i+1)"
            $previousText=[IO.File]::ReadAllText($fixture)
            Invoke-TexziWindow @{action='text';label="$name-edit";text=$probe}
            Invoke-TexziWindow @{action='keys';label="$name-save";keys=@(17,83)}
            $saved=[IO.File]::ReadAllText($fixture)
            $position=$saved.IndexOf($probe,[StringComparison]::Ordinal)
            $editable=$position -ge 0 -and !$previousText.Contains($probe) -and
                $saved.Remove($position,$probe.Length) -ceq $previousText
        }
        $rows += [pscustomobject]@{name=$name;entry=$entry;focus=$focus;focus_probe=$focusProbe;edit_persisted=$editable}
    }
    Invoke-TexziWindow @{action='close';label='98-close'}
    [void]$driver.WaitForExit(10000)
    $result.driver_exit=$driver.ExitCode
    $result.app=Get-Content -Raw -LiteralPath "$run/result.json" | ConvertFrom-Json
    $result.saved_fixture_sha256=(Get-FileHash -LiteralPath $fixture).Hash
    $env:TEXZI_OCR_DIR=$run
    $env:TEXZI_OCR_PATTERN='-(before|after)\.png$|^(92|93|96-panel|97)-'
    $env:TEXZI_OCR_LANG='zh-Hans-CN'
    $env:TEXZI_OCR_SCRIPT="$previous/ocr-candidate.ps1"
    $out=& "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" -NoProfile -NonInteractive -Command '& ([scriptblock]::Create([IO.File]::ReadAllText($env:TEXZI_OCR_SCRIPT))) -Directory $env:TEXZI_OCR_DIR'
    if($LASTEXITCODE -ne 0){throw 'Could not read test screenshots'}
    $out|Set-Content -LiteralPath "$run/ocr.json" -Encoding utf8
    $ocr=$out|Out-String|ConvertFrom-Json
    $result.rows=@(foreach($row in $rows){
        $before=($ocr|Where-Object file -EQ "$($row.name)-before.png").text -replace '\s',''
        $after=($ocr|Where-Object file -EQ "$($row.name)-after.png").text -replace '\s',''
        $row|Add-Member NoteProperty panel_before ($before -match '替换为|全部替换|替换当前')
        $row|Add-Member NoteProperty panel_after ($after -match '替换为|全部替换|替换当前')
        $row|Add-Member NoteProperty focus_probe_visible ([bool]$row.focus_probe -and $before.Contains($row.focus_probe))
        $row
    })
    if($result.driver_exit -ne 0 -or $result.app.exit -ne 0 -or !$result.rows -or
       ($result.rows | Where-Object { !$_.panel_before -or $_.panel_after -ne [bool]$Baseline -or
        (!$Baseline -and !$_.edit_persisted) -or ($_.focus -eq 'find' -and !$_.focus_probe_visible) })){
        throw 'Escape observations did not match expectations'
    }
} finally {
    if($null -ne $driver -and !$driver.HasExited -and !(Test-Path "$run/result.json")){
        try{Invoke-TexziWindow @{action='abort';label='cleanup-owned-window'}}catch{Write-Warning $_.ToString()}
    }
    $mutex.ReleaseMutex()
    $mutex.Dispose()
    $fileName=if($Baseline){'baseline.json'}else{'escape-matrix.json'}
    $result|ConvertTo-Json -Depth 7|Tee-Object -FilePath (Join-Path $PSScriptRoot $fileName)
}


