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

$temp = [IO.Path]::TrimEndingDirectorySeparator([IO.Path]::GetFullPath([IO.Path]::GetTempPath()))
$scratch = Join-Path $temp ("texzi-startup-" + [guid]::NewGuid())
$local = Join-Path $scratch 'local'
$roaming = Join-Path $scratch 'roaming'
$folder = Join-Path $scratch 'fixtures'
$recovery = Join-Path $local 'texzi/recovery'
foreach ($directory in $folder, $roaming, $recovery) { [IO.Directory]::CreateDirectory($directory) | Out-Null }
$settings = Join-Path $local 'texzi/settings.json'
[IO.File]::WriteAllText($settings, '{"version":1,"user":{"files.autoSave":"afterDelay"},"folders":{}}')
$markdown = Join-Path $folder 'body.md'
$markdownBody = "# TEXZI-STARTUP-HEADING`n`nTEXZI-STARTUP-BODY`n"
[IO.File]::WriteAllText($markdown, $markdownBody)
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
function Find-OnlyEditor {
    $edits = $workbench.FindAll([Windows.Automation.TreeScope]::Descendants, $editCondition)
    $found = $null
    foreach ($edit in $edits) {
        $value = $null
        if ($edit.Current.IsEnabled -and $edit.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$value) -and !$value.Current.IsReadOnly) {
            if ($null -ne $found) { return $null }
            $found = $value
        }
    }
    return $found
}
function Edit-AndWaitForSave([string]$FileName) {
    # The editor's input sink can have an empty UIA Value beside the rendered text.
    # A write in the isolated fixture verifies which document the sink actually edits.
    $editor = Wait-For "the editable body of $FileName" { Find-OnlyEditor }
    $editor.SetValue('~')
    Wait-For "the edit saved to $FileName" { Has-Text "已保存 $FileName" } | Out-Null
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
    Wait-For 'Markdown source' { (Find-Named '返回所见即所得') -and (Has-Text 'TEXZI-STARTUP-HEADING') } | Out-Null
    Edit-AndWaitForSave 'body.md'
    $saved = [IO.File]::ReadAllText($markdown)
    if ($saved.Length -ne $markdownBody.Length + 1 -or $saved.Replace('~', '') -cne $markdownBody) {
        throw 'The Markdown source edit did not preserve the rest of the document.'
    }
    Invoke-Named '返回所见即所得'
    Wait-For 'formatted Markdown body' { (Find-Named '格式命令') -and (Has-Text 'TEXZI-STARTUP-BODY') } | Out-Null
    Invoke-Named 'large.txt'
    Wait-For 'large-file import to finish' { (Has-Text '大文件加载完成') -and (Has-Text 'TEXZI-LARGE-BODY') } | Out-Null
    Edit-AndWaitForSave 'large.txt'
    $stream = [IO.File]::OpenRead($large)
    try {
        if ($stream.ReadByte() -ne [int][char]'~' -or [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)) -ne $original) {
            throw 'The 20 MiB edit did not add the expected byte and preserve the complete original text.'
        }
    } finally { $stream.Dispose() }
    if (![TexziStartup]::Close($handle)) { throw 'Failed to request close on the workbench.' }
    if (!$proc.WaitForExit(15000) -or $proc.ExitCode -ne 0) { throw 'The workbench did not exit cleanly.' }
    Write-Host "Verified Windows workbench PID $($proc.Id), Markdown modes, on-disk edits to both fixtures and clean exit."
} finally {
    if (!$proc.HasExited) {
        $proc.Kill()
        if (!$proc.WaitForExit(5000)) { throw "The isolated process $($proc.Id) did not stop." }
    }
    if ($stderr.IsCompletedSuccessfully) { Write-Host $stderr.Result }
    $proc.Dispose()
}
$owned = Get-Item -LiteralPath $scratch
if ($owned.Parent.FullName -ne $temp -or $owned.FullName -ne $scratch) { throw "Unexpected fixture directory $scratch" }
Remove-Item -LiteralPath $owned.FullName -Recurse -Force
