function Invoke-TexziWindow([hashtable]$Command) {
    $ErrorActionPreference = 'Stop'
    $label = $Command.label
    if (!(Test-Path "$run/ready.json")) { throw 'No owned window ready' }
    if (Test-Path "$run/result.json") { throw (Get-Content "$run/result.json" -Raw) }
    if (Test-Path "$run/command.json") { throw 'Previous command is still pending' }
    $Command | ConvertTo-Json -Compress | Set-Content "$run/command-ready.json"
    Move-Item -LiteralPath "$run/command-ready.json" -Destination "$run/command.json"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while (!(Test-Path "$run/ack-$label.json")) {
        if (Test-Path "$run/result.json") {
            $result = Get-Content "$run/result.json" -Raw | ConvertFrom-Json
            if ($result.status -eq 'PASS' -and ($Command.action -in @('close','keys-close','discard-close') -or $Command.closes)) {return}
            throw "Window stopped at $label with $($result.status): $($result.failure)"
        }
        if ($sw.ElapsedMilliseconds -ge 102000) { throw "Timeout waiting for $label" }
        Start-Sleep -Milliseconds 100
    }
}



