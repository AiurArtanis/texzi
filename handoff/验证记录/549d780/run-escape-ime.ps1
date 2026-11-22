$ErrorActionPreference='Stop'
$candidate=Get-Content -Raw -LiteralPath "$PSScriptRoot/candidate-identity.json" | ConvertFrom-Json
$previous=$PSScriptRoot
. "$previous/window-command.ps1"
$run=Join-Path $PSScriptRoot "ime-round"
$mutex=[Threading.Mutex]::new($false,'Local\Texzi.Paladin.DesktopTest')
if(!$mutex.WaitOne(30000)){throw 'Paladin desktop already in use'}
$result=[ordered]@{commit=$candidate.commit;exe_sha256=$candidate.exe_sha256;run=$run}
try {
    $driver=Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @(
        '-NoProfile','-File',"$previous/window-driver.ps1",'-Round',$run,'-Mode','trace',
        '-Exe',$candidate.unpacked_exe,'-Commit',$candidate.commit,'-Hash',$candidate.exe_sha256
    ) -WindowStyle Hidden -PassThru
    $deadline=[DateTime]::UtcNow.AddSeconds(30)
    while(!(Test-Path "$run/ready.json") -and !$driver.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 125}
    Invoke-TexziWindow @{action='resize';label='00-wide';x=25;y=25;w=1774;h=770}
    Invoke-TexziWindow @{action='keys';label='01-open';keys=@(17,72)}
    Invoke-TexziWindow @{action='physical';label='02-preedit';keys=@(78,73)}
    $before=Get-Content -Raw -LiteralPath "$run/stderr.log"
    $result.preedit_observed=$before -match 'TX04_IME_NATIVE_EVENT composing=true preedit_utf16=[1-9]'
    if($result.preedit_observed){
        Invoke-TexziWindow @{action='keys';label='03-first-escape';keys=@(27)}
        $after=Get-Content -Raw -LiteralPath "$run/stderr.log"
        $result.cancellation_observed=$after.Substring($before.Length) -match 'TX04_IME_NATIVE_EVENT composing=false preedit_utf16=0'
        Invoke-TexziWindow @{action='keys';label='04-second-escape';keys=@(27)}
    }
    Invoke-TexziWindow @{action='close';label='05-close'}
    [void]$driver.WaitForExit(10000)
    $result.driver_exit=$driver.ExitCode
    $result.app_exit=(Get-Content -Raw -LiteralPath "$run/result.json" | ConvertFrom-Json).exit
    if($result.preedit_observed){
        $env:TEXZI_OCR_DIR=$run
        $env:TEXZI_OCR_PATTERN='^(03|04)-.*\.png$'
        $env:TEXZI_OCR_LANG='zh-Hans-CN'
        $env:TEXZI_OCR_SCRIPT="$previous/ocr-candidate.ps1"
        $out=& "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" -NoProfile -NonInteractive -Command '& ([scriptblock]::Create([IO.File]::ReadAllText($env:TEXZI_OCR_SCRIPT))) -Directory $env:TEXZI_OCR_DIR'
        if($LASTEXITCODE -ne 0){throw 'Could not read the IME screenshots'}
        $ocr=$out|Out-String|ConvertFrom-Json
        $result.panel_after_first=($ocr|Where-Object file -EQ '03-first-escape.png').text -replace '\s','' -match '替换为|全部替换|替换当前'
        $result.panel_after_second=($ocr|Where-Object file -EQ '04-second-escape.png').text -replace '\s','' -match '替换为|全部替换|替换当前'
        if(!$result.cancellation_observed -or !$result.panel_after_first -or $result.panel_after_second){throw 'Unexpected Escape behavior during IME composition'}
    }
    if($result.driver_exit -ne 0 -or $result.app_exit -ne 0){throw 'Test window did not exit naturally'}
} finally {
    if($null -ne $driver -and !$driver.HasExited -and !(Test-Path "$run/result.json")){
        try{Invoke-TexziWindow @{action='abort';label='cleanup-owned-window'}}catch{Write-Warning $_.ToString()}
    }
    $mutex.ReleaseMutex()
    $mutex.Dispose()
    $result|ConvertTo-Json|Tee-Object -FilePath "$PSScriptRoot/escape-ime.json"
}

