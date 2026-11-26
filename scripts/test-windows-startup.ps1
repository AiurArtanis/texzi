param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TexziStartup {
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int left, top, right, bottom; }
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr window, int key, out int value, int size);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    public static bool Visible(IntPtr window) =>
        IsWindowVisible(window) && GetClientRect(window, out var r) && r.right > r.left && r.bottom > r.top &&
        DwmGetWindowAttribute(window, 14, out var cloaked, 4) == 0 && cloaked == 0;
    public static bool Close(IntPtr window) => PostMessageW(window, 0x0010, IntPtr.Zero, IntPtr.Zero);
}
'@

$scratch = Join-Path ([IO.Path]::GetTempPath()) ("texzi-startup-" + [guid]::NewGuid())
$local = Join-Path $scratch 'local'
$roaming = Join-Path $scratch 'roaming'
$folder = Join-Path $scratch 'fixtures'
$recovery = Join-Path $local 'texzi/recovery'
foreach ($directory in $folder, $roaming, $recovery) { [IO.Directory]::CreateDirectory($directory) | Out-Null }
$markdown = Join-Path $folder 'body.md'
[IO.File]::WriteAllText($markdown, "# TEXZI-STARTUP-HEADING`n`nTEXZI-STARTUP-BODY`n")
$large = Join-Path $folder 'large.txt'
$block = [Text.Encoding]::ASCII.GetBytes((("TEXZI-LARGE-BODY".PadRight(127, '.') + "`n") * 512))
$stream = [IO.File]::Open($large, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
try {
    for ($i = 0; $i -lt 320; $i++) { $stream.Write($block, 0, $block.Length) }
} finally { $stream.Dispose() }
$original = (Get-FileHash -LiteralPath $large).Hash
$tab = @{ id=1; path=[int[]]$markdown.ToCharArray(); view=@{rich=$true}; draft=$null; blank=$null }
$session = @{version=1; folder=[int[]]$folder.ToCharArray(); sidebar=$true; sidebar_width=240; active=1; tabs=@($tab); split=$null}
[IO.File]::WriteAllText((Join-Path $recovery '900000001.json'), ($session | ConvertTo-Json -Depth 6 -Compress))

function Wait-For([string]$Description, [scriptblock]$Observe) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.Elapsed.TotalSeconds -lt 30) {
        if ($proc.HasExited) { throw "texzi exited with $($proc.ExitCode) while waiting for $Description" }
        try { $result = & $Observe } catch [System.Windows.Automation.ElementNotAvailableException] { $result = $null }
        if ($result) { return $result }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for $Description"
}
function Find-Named([string]$Name) {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $Name)
    $workbench.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Invoke-Named([string]$Name) {
    $element = Wait-For $Name { $node = Find-Named $Name; if ($node -and $node.Current.IsEnabled) { $node } }
    $element.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Has-Text([string]$Text) {
    $nodes = $workbench.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
    foreach ($node in $nodes) {
        if ($node.Current.Name -and $node.Current.Name.Contains($Text)) { return $true }
    }
    return $false
}
function Find-Editor([string]$Marker) {
    $edits = $workbench.FindAll([Windows.Automation.TreeScope]::Descendants, $editCondition)
    foreach ($edit in $edits) {
        $value = $null
        if ($edit.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$value) -and
            !$value.Current.IsReadOnly -and ($value.Current.Value.Contains($Marker) -or (Has-Text $Marker))) { return $value }
    }
}

$start = [Diagnostics.ProcessStartInfo]::new((Resolve-Path -LiteralPath $Exe).Path)
$start.UseShellExecute = $false
$start.WorkingDirectory = $folder
$start.Environment['LOCALAPPDATA'] = $local
$start.Environment['APPDATA'] = $roaming
$start.RedirectStandardError = $true
$proc = [Diagnostics.Process]::Start($start)
$stderr = $proc.StandardError.ReadToEndAsync()
try {
    $mine = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $proc.Id)
    $editCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit)
    $workbench = Wait-For 'a visible workbench' {
        $roots = [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $mine)
        $visible = @($roots | Where-Object { $_.Current.Name -eq 'texzi — 原生编辑' -and [TexziStartup]::Visible($_.Current.NativeWindowHandle) })
        if ($visible.Count -eq 1) { $visible[0] }
    }
    $handle = [IntPtr]$workbench.Current.NativeWindowHandle
    Wait-For 'the workbench search' { Find-Named '搜索文件或命令' } | Out-Null
    Invoke-Named '切换到源码'
    Wait-For 'editable Markdown source' { Find-Editor 'TEXZI-STARTUP-HEADING' } | Out-Null
    Invoke-Named '返回所见即所得'
    Wait-For 'formatted Markdown' { Find-Named '格式命令' } | Out-Null
    Invoke-Named 'large.txt'
    Wait-For 'large-file import to finish' { Has-Text '大文件加载完成' } | Out-Null
    $editor = Wait-For 'the complete 20 MiB editable body' { Find-Editor 'TEXZI-LARGE-BODY' }
    if ($editor.Current.IsReadOnly) { throw 'The large-file preview did not reach full edit mode.' }
    if (![TexziStartup]::Close($handle)) { throw 'Failed to request close on the workbench.' }
    if (!$proc.WaitForExit(15000) -or $proc.ExitCode -ne 0) { throw 'The workbench did not exit cleanly.' }
    if ((Get-FileHash -LiteralPath $large).Hash -ne $original) { throw 'Opening the 20 MiB fixture changed it on disk.' }
    Write-Host "Verified Windows workbench PID $($proc.Id), Markdown modes, editable 20 MiB and clean exit."
} finally {
    if (!$proc.HasExited) { $proc.Kill(); $proc.WaitForExit() }
    if ($stderr.IsCompletedSuccessfully) { Write-Host $stderr.Result }
    $proc.Dispose()
}
