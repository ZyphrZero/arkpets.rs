param(
    [string]$Executable = "$PSScriptRoot/../../target/debug/arkpets-ui.exe",
    [ValidateSet('dx12', 'opengl')][string]$Renderer = 'dx12',
    [ValidateRange(1, 20)][int]$Switches = 4,
    [switch]$DesktopPet,
    [string]$Shell = 'pwsh',
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
if (-not ('ArkRendererSwitch' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class ArkRendererSwitch {
    public delegate bool Visit(IntPtr window, IntPtr context);
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct MouseInput { public int X, Y; public uint Data, Flags, Time; public UIntPtr Extra; }
    [StructLayout(LayoutKind.Explicit)] public struct InputUnion { [FieldOffset(0)] public MouseInput Mouse; }
    [StructLayout(LayoutKind.Sequential)] public struct Input { public uint Type; public InputUnion Value; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(Visit visit, IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder title, int length);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint count, Input[] inputs, int size);
    public static IntPtr FindWindow(uint processId, bool pet) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, context) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var title = new StringBuilder(256);
            GetWindowText(window, title, title.Capacity);
            if (pet ? title.ToString().StartsWith("ArkPets - ") : title.ToString() == "ArkPets") {
                found = window;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static void LeftButton(bool down) {
        var input = new Input();
        input.Value.Mouse.Flags = down ? 0x0002u : 0x0004u;
        if (SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Input))) != 1)
            throw new InvalidOperationException("SendInput failed: " + Marshal.GetLastWin32Error());
    }
}
'@
}

$repository = (Resolve-Path -LiteralPath "$PSScriptRoot/../..").Path
$exe = (Resolve-Path -LiteralPath $Executable).Path
$shellPath = (Get-Command $Shell -ErrorAction Stop).Source
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repository ('target/renderer-switch/' + [Guid]::NewGuid().ToString('N'))
}
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($outputPath) | Out-Null
$reportPath = Join-Path $outputPath 'switch.json'
if (Test-Path -LiteralPath $reportPath) { throw "An existing report would be overwritten: $reportPath" }
$utf8 = [Text.UTF8Encoding]::new($false)
$samples = [Collections.Generic.List[object]]::new()
$report = [ordered]@{ renderer = $Renderer; desktop_pet = [bool]$DesktopPet; success = $false; switches = $samples }
$shellProcess = [Diagnostics.Process]::new()
$appProcess = $null
$started = $false
$originalCursor = New-Object ArkRendererSwitch+Point
$previousDpi = [ArkRendererSwitch]::SetThreadDpiAwarenessContext([IntPtr](-4))
if (-not [ArkRendererSwitch]::GetCursorPos([ref]$originalCursor)) { throw 'Cannot read cursor position' }
if (([ArkRendererSwitch]::GetAsyncKeyState(1) -band 0x8000) -ne 0) { throw 'The left mouse button is already held' }

function Assert-SessionRunning {
    if ($shellProcess.HasExited) { throw 'PowerShell returned before the application was closed' }
    if ($appProcess -and $appProcess.HasExited) { throw 'Renderer switch terminated the original application process' }
}

function Wait-Window([bool]$Pet, [IntPtr]$Previous = [IntPtr]::Zero) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-SessionRunning
        $window = [ArkRendererSwitch]::FindWindow([uint32]$appProcess.Id, $Pet)
        if ($window -ne [IntPtr]::Zero -and $window -ne $Previous) { return $window }
        Start-Sleep -Milliseconds 25
    } while ($timer.ElapsedMilliseconds -lt 10000)
    throw "Timed out waiting for a recreated window (pet=$Pet) in PID $($appProcess.Id)"
}

function Show-Root([IntPtr]$Window, [bool]$Resize = $false) {
    [ArkRendererSwitch]::ShowWindow($Window, 4) | Out-Null
    $scale = [ArkRendererSwitch]::GetDpiForWindow($Window) / 96.0
    $flags = if ($Resize) { 0x10 } else { 0x13 }
    if (-not [ArkRendererSwitch]::SetWindowPos($Window, [IntPtr](-1), 30, 20, [int](1200 * $scale), [int](1000 * $scale), $flags)) {
        throw 'Cannot position the sample window'
    }
    Start-Sleep -Milliseconds 300
}

function Click-Client([IntPtr]$Window, [double]$X, [double]$Y) {
    $scale = [ArkRendererSwitch]::GetDpiForWindow($Window) / 96.0
    $point = New-Object ArkRendererSwitch+Point
    $point.X = [int][Math]::Round($X * $scale)
    $point.Y = [int][Math]::Round($Y * $scale)
    if (-not [ArkRendererSwitch]::ClientToScreen($Window, [ref]$point)) { throw 'ClientToScreen failed' }
    if (-not [ArkRendererSwitch]::SetCursorPos($point.X, $point.Y)) { throw 'SetCursorPos failed' }
    Start-Sleep -Milliseconds 80
    $target = [ArkRendererSwitch]::GetAncestor([ArkRendererSwitch]::WindowFromPoint($point), 2)
    if ($target -ne $Window) {
        throw "Another window covers the sample at client ($X,$Y), screen ($($point.X),$($point.Y)): expected HWND $Window, found $target; no click was sent"
    }
    [ArkRendererSwitch]::LeftButton($true)
    try { Start-Sleep -Milliseconds 80 } finally { [ArkRendererSwitch]::LeftButton($false) }
    Start-Sleep -Milliseconds 180
}

try {
    # Use a real PowerShell native-command invocation. The marker represents the prompt returning.
    $command = "& '" + $exe.Replace("'", "''") + "' --renderer $Renderer; " +
        '$code = $LASTEXITCODE; Write-Output ''TERMINAL_COMMAND_RETURNED''; exit $code'
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $shellPath
    $start.Arguments = "-NoProfile -EncodedCommand $encoded"
    $start.WorkingDirectory = $repository
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.EnvironmentVariables['ARKPETS_PROFILE_DIR'] = $outputPath
    $start.EnvironmentVariables['ARKPETS_PROFILE_SECONDS'] = '120'
    $start.EnvironmentVariables['ARKPETS_PROFILE_WARMUP'] = '2'
    $start.EnvironmentVariables['ARKPETS_PROFILE_MODEL'] = 'pepe'
    $start.EnvironmentVariables['ARKPETS_PROFILE_FPS'] = '60'
    $start.EnvironmentVariables.Remove('ARKPETS_CAPTURE_DIR')
    $shellProcess.StartInfo = $start
    if (-not $shellProcess.Start()) { throw 'Cannot start the PowerShell sample' }
    $started = $true
    $null = $shellProcess.Handle
    $stderr = $shellProcess.StandardError.ReadToEndAsync()
    $stdout = $shellProcess.StandardOutput.ReadToEndAsync()
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-SessionRunning
        $child = Get-CimInstance Win32_Process -Filter "Name='arkpets-ui.exe' AND ParentProcessId=$($shellProcess.Id)"
        if ($child) { $appProcess = [Diagnostics.Process]::GetProcessById($child.ProcessId); break }
        Start-Sleep -Milliseconds 50
    } while ($timer.ElapsedMilliseconds -lt 10000)
    if (-not $appProcess) { throw 'PowerShell did not launch the sample application' }
    $null = $appProcess.Handle
    $report.process_id = $appProcess.Id
    $root = Wait-Window $false
    Start-Sleep -Milliseconds 700
    Show-Root $root $true
    Click-Client $root 100 156 # Settings page.
    $null = Wait-Window $true
    if (-not $DesktopPet) {
        $rect = New-Object ArkRendererSwitch+Rect
        if (-not [ArkRendererSwitch]::GetClientRect($root, [ref]$rect)) { throw 'Cannot read ROOT client rectangle' }
        $scale = [ArkRendererSwitch]::GetDpiForWindow($root) / 96.0
        Click-Client $root ($rect.Right / $scale - 55) 36
    }
    $current = $Renderer
    for ($index = 0; $index -lt $Switches; $index++) {
        $next = if ($current -eq 'dx12') { 'opengl' } else { 'dx12' }
        $timer.Restart()
        # The settings page must survive each switch; select without an Apply button.
        Click-Client $root 300 827
        $row = if ($next -eq 'dx12') { 866 } else { 908 }
        Click-Client $root 300 $row
        $root = Wait-Window $false $root
        Show-Root $root
        if ($DesktopPet) {
            $pet = Wait-Window $true
            $visibleTimer = [Diagnostics.Stopwatch]::StartNew()
            while (-not [ArkRendererSwitch]::IsWindowVisible($pet)) {
                Assert-SessionRunning
                if ($visibleTimer.ElapsedMilliseconds -gt 3000) { throw 'Recreated desktop pet remained hidden' }
                Start-Sleep -Milliseconds 25
            }
        }
        Assert-SessionRunning
        $saved = Get-Content -LiteralPath (Join-Path $outputPath 'bootstrap.json') -Raw | ConvertFrom-Json
        if ($saved.renderer -ne $next) { throw "Backend preference was not saved as $next" }
        $sample = [pscustomobject]@{ from = $current; to = $next; elapsed_ms = $timer.ElapsedMilliseconds; process_id = $appProcess.Id }
        $samples.Add($sample)
        Write-Output "Switch PASS: $current -> $next, PID $($appProcess.Id), $($sample.elapsed_ms) ms"
        $current = $next
    }
    $timer.Restart()
    if (-not [ArkRendererSwitch]::PostMessage($root, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'WM_CLOSE failed' }
    if (-not $appProcess.WaitForExit(3000)) { throw 'Application did not exit within three seconds' }
    if (-not $shellProcess.WaitForExit(3000)) { throw 'PowerShell did not finish after application exit' }
    if ($appProcess.ExitCode -ne 0 -or $shellProcess.ExitCode -ne 0) { throw 'Application or PowerShell exited with an error' }
    $errorText = $stderr.GetAwaiter().GetResult()
    $outputText = $stdout.GetAwaiter().GetResult()
    if ($outputText.Trim() -ne 'TERMINAL_COMMAND_RETURNED') { throw 'PowerShell did not return exactly once' }
    $renderers = @([regex]::Matches($errorText, 'Renderer: [^\r\n]+ / (Dx12|OpenGL)') | ForEach-Object { $_.Groups[1].Value.ToLowerInvariant() })
    $expected = @($Renderer) + @($samples | ForEach-Object { $_.to })
    if (($renderers -join ',') -ne ($expected -join ',')) { throw "GPU backend sequence differs: $($renderers -join ',')" }
    $report.exit_elapsed_ms = $timer.ElapsedMilliseconds
    $report.success = $true
    Write-Output "Exit PASS: application and PowerShell finished in $($report.exit_elapsed_ms) ms"
} catch {
    $report.failure = $_.Exception.Message
    throw
} finally {
    [ArkRendererSwitch]::SetCursorPos($originalCursor.X, $originalCursor.Y) | Out-Null
    if ($previousDpi -ne [IntPtr]::Zero) { [ArkRendererSwitch]::SetThreadDpiAwarenessContext($previousDpi) | Out-Null }
    if ($started) {
        # Only terminate this test's process tree after recording a failed assertion.
        # A regressed build may have exited and left a replacement process holding the pipes.
        if ($appProcess) {
            $replacements = Get-CimInstance Win32_Process -Filter "Name='arkpets-ui.exe' AND ParentProcessId=$($appProcess.Id)"
            foreach ($replacement in $replacements) {
                if ($replacement.ExecutablePath -eq $exe -and $replacement.CreationDate -ge $shellProcess.StartTime) {
                    $orphan = [Diagnostics.Process]::GetProcessById($replacement.ProcessId)
                    try { $orphan.Kill($true); $orphan.WaitForExit() } finally { $orphan.Dispose() }
                }
            }
        }
        if (-not $shellProcess.HasExited) { $shellProcess.Kill($true); $shellProcess.WaitForExit() }
        if ($appProcess -and -not $appProcess.HasExited) { $appProcess.Kill($true); $appProcess.WaitForExit() }
        [IO.File]::WriteAllText((Join-Path $outputPath 'stderr.log'), $stderr.GetAwaiter().GetResult(), $utf8)
        [IO.File]::WriteAllText((Join-Path $outputPath 'stdout.log'), $stdout.GetAwaiter().GetResult(), $utf8)
    }
    [IO.File]::WriteAllText($reportPath, ($report | ConvertTo-Json -Depth 5), $utf8)
    if ($appProcess) { $appProcess.Dispose() }
    $shellProcess.Dispose()
    Write-Output "Renderer switch report: $reportPath"
}
