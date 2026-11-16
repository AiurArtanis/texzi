param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$EvidenceDir)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$exePath = (Resolve-Path -LiteralPath $Exe).Path
if (Test-Path -LiteralPath $EvidenceDir) { throw 'Use a new evidence directory; historical evidence must remain intact.' }
New-Item -Path $EvidenceDir -ItemType Directory | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $EvidenceDir).Path
$fixtures = Join-Path $outputRoot 'fixtures'
New-Item $fixtures -ItemType Directory | Out-Null
[IO.File]::WriteAllText((Join-Path $fixtures 'unicode.txt'), "中文 UTF-8`n")
[IO.File]::WriteAllText((Join-Path $fixtures 'note.md'), "# fixture`nunchanged`n")
[IO.File]::WriteAllBytes((Join-Path $fixtures 'empty.txt'), [byte[]]@())
[IO.File]::WriteAllBytes((Join-Path $fixtures 'binary.bin'), [byte[]](0..255))
function Snapshot {
    @(Get-ChildItem $fixtures -Recurse -File | Sort-Object FullName | ForEach-Object {
        [ordered]@{ path=$_.FullName.Substring($fixtures.Length); length=$_.Length;
            sha256=(Get-FileHash -LiteralPath $_.FullName).Hash; mtime=$_.LastWriteTimeUtc.ToString('o') }
    }) | ConvertTo-Json -Depth 5
}
$before = Snapshot
$before | Set-Content (Join-Path $outputRoot 'fixtures-before.json')
Get-FileHash $exePath | ConvertTo-Json | Set-Content (Join-Path $outputRoot 'exe-hash.json')
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class Tx02Dialog {
 delegate bool Visitor(IntPtr hwnd, IntPtr data);
 [DllImport("user32.dll")] static extern bool EnumWindows(Visitor visitor, IntPtr data);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder text, int size);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int size);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd,uint message,IntPtr wparam,IntPtr lparam);
 public static IntPtr Find(int expectedPid) {
   IntPtr found=IntPtr.Zero;
   EnumWindows((hwnd, data) => {
     uint pid; GetWindowThreadProcessId(hwnd,out pid);
     if(pid != expectedPid) return true;
     var cls=new StringBuilder(256); var title=new StringBuilder(256);
     GetClassName(hwnd,cls,256); GetWindowText(hwnd,title,256);
     if(cls.ToString()=="#32770" && title.ToString()=="nsh") {found=hwnd;return false;}
     return true;
   },IntPtr.Zero);
   return found;
 }
}
'@
$timer = [Diagnostics.Stopwatch]::StartNew()
$process = Start-Process $exePath -ArgumentList '--tx00-fail-window-create' -WorkingDirectory $fixtures -PassThru -RedirectStandardOutput (Join-Path $outputRoot 'stdout.log') -RedirectStandardError (Join-Path $outputRoot 'stderr.log')
$observation = [ordered]@{ pid=$process.Id; modal_seen=$false; modal_dismissal='NOT_DONE'; exit_code=$null; residual=$null; elapsed_ms=$null }
try {
    $dialog = [IntPtr]::Zero
    do {
        $process.Refresh()
        if ($process.HasExited) { break }
        $dialog = [Tx02Dialog]::Find($process.Id)
        if ($dialog -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    } while ($timer.Elapsed.TotalSeconds -lt 30)
    if ($dialog -eq [IntPtr]::Zero) { throw 'Expected NSH native modal error dialog not observed within 30s' }
    $observation.modal_seen = $true
    $observation.modal_hwnd = $dialog.ToInt64()
    $observation.modal_seen_ms = $timer.ElapsedMilliseconds
    # Explicitly acknowledge this test PID's own NSH MessageBox (IDOK).
    # This is not a claim that the application exits without interaction.
    if (![Tx02Dialog]::PostMessage($dialog, 0x0111, [IntPtr]1, [IntPtr]::Zero)) { throw 'Could not acknowledge NSH error dialog' }
    $observation.modal_dismissal = 'WM_COMMAND IDOK sent to matched NSH dialog for test PID'
    if (!$process.WaitForExit(15000)) { throw 'No process exit within 15s after modal acknowledgement' }
    $observation.exit_code = $process.ExitCode
    $observation.elapsed_ms = $timer.ElapsedMilliseconds
    $observation.residual = [bool](Get-Process -Id $process.Id -ErrorAction SilentlyContinue)
    if ($process.ExitCode -eq 0 -or $observation.residual) { throw 'Expected nonzero exit and no residual test PID' }
    $stderr = Get-Content (Join-Path $outputRoot 'stderr.log') -Raw
    if ($stderr -notmatch 'rejecting real NshWindow WM_NCCREATE' -or $stderr -notmatch 'Cannot open a window:') { throw 'Missing real OS/NSH initialization failure diagnostic' }
} finally {
    if (!$process.HasExited) {
        $observation.forced_cleanup = $true
        Stop-Process -Id $process.Id
    }
    $after = Snapshot
    $after | Set-Content (Join-Path $outputRoot 'fixtures-after.json')
    $observation.snapshot_equal = ($before -eq $after)
    $observation | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $outputRoot 'observation.json')
}
if ($before -ne $after) { throw 'Fixture path/length/hash/mtime changed' }
$observation | ConvertTo-Json -Depth 5
