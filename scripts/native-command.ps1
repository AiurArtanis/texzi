# External commands are captured directly, independently of PowerShell transcription.
function Invoke-Checked {
    param(
        [Parameter(Mandatory)][string]$Program,
        [string[]]$Arguments = @(),
        [string]$StdoutFile
    )
    $sequence = @(Get-ChildItem build-evidence -Filter '*.result.json').Count + 1
    $prefix = Join-Path (Resolve-Path build-evidence).Path ('command-{0:D3}' -f $sequence)
    $command = 'COMMAND: ' + $Program + ' ' + ($Arguments -join ' ')
    Add-Content build-evidence/build.log $command
    Write-Host $command
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Program
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $start.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (!$process.Start()) { throw "Could not start $Program" }
        # Drain both streams concurrently: waiting on either one first can deadlock.
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $code = $process.ExitCode
        [IO.File]::WriteAllText("$prefix.stdout.log", $stdout)
        [IO.File]::WriteAllText("$prefix.stderr.log", $stderr)
        if ($StdoutFile) { [IO.File]::WriteAllText((Join-Path $PWD $StdoutFile), $stdout) }
        [ordered]@{ program=$Program; arguments=$Arguments; exit_code=$code;
            stdout_file=(Split-Path "$prefix.stdout.log" -Leaf);
            stderr_file=(Split-Path "$prefix.stderr.log" -Leaf) } |
            ConvertTo-Json -Depth 4 | Set-Content "$prefix.result.json"
        Add-Content build-evidence/build.log "STDOUT:`n$stdout`nSTDERR:`n$stderr`nEXIT: $code"
        Write-Host $stdout
        Write-Host $stderr
        Write-Host "EXIT: $code"
        if ($code -ne 0) { throw "$Program failed: $code" }
    } finally {
        $process.Dispose()
    }
}
