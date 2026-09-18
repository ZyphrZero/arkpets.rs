param(
    [string]$Executable = "$PSScriptRoot/../../target/debug/arkpets-ui.exe",
    [ValidateSet('dx12', 'opengl')][string]$Renderer = 'dx12',
    [switch]$DesktopSettings,
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
if (-not ('ArkPetsInteraction' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class ArkPetsInteraction {
    public delegate bool Visit(IntPtr window, IntPtr context);
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct MonitorInfo { public int Size; public Rect Monitor, Work; public uint Flags; }
    [StructLayout(LayoutKind.Sequential)] public struct MouseInput { public int X, Y; public uint Data, Flags, Time; public UIntPtr Extra; }
    [StructLayout(LayoutKind.Sequential)] public struct KeyboardInput { public ushort Key, Scan; public uint Flags, Time; public UIntPtr Extra; }
    [StructLayout(LayoutKind.Explicit)] public struct InputUnion { [FieldOffset(0)] public MouseInput Mouse; [FieldOffset(0)] public KeyboardInput Keyboard; }
    [StructLayout(LayoutKind.Sequential)] public struct Input { public uint Type; public InputUnion Value; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(Visit visit, IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder title, int length);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr window, int index);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
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
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr window, uint flags);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    public static IntPtr FindWindow(uint processId, bool pet) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, context) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var title = new StringBuilder(256);
            GetWindowText(window, title, title.Capacity);
            bool matches = pet ? title.ToString().StartsWith("ArkPets - ") : title.ToString() == "ArkPets";
            if (!matches) return true;
            found = window;
            return false;
        }, IntPtr.Zero);
        return found;
    }
    public static void LeftButton(bool down) {
        var input = new Input();
        input.Value.Mouse.Flags = down ? 0x0002u : 0x0004u;
        if (SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Input))) != 1)
            throw new InvalidOperationException("SendInput failed: " + Marshal.GetLastWin32Error());
    }
    public static void Wheel(int delta) {
        var input = new Input();
        input.Value.Mouse.Flags = 0x0800;
        input.Value.Mouse.Data = unchecked((uint)delta);
        if (SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Input))) != 1)
            throw new InvalidOperationException("Wheel SendInput failed");
    }
    public static void Key(ushort key, bool down) {
        var input = new Input();
        input.Type = 1;
        input.Value.Keyboard.Key = key;
        input.Value.Keyboard.Flags = down ? 0u : 2u;
        if (SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Input))) != 1)
            throw new InvalidOperationException("Keyboard SendInput failed");
    }
}
'@
}

$repository = (Resolve-Path -LiteralPath "$PSScriptRoot/../..").Path
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path ([IO.Path]::GetTempPath()) ('arkpets-interaction-' + [Guid]::NewGuid().ToString('N'))
}
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($outputPath) | Out-Null
$reportPath = Join-Path $outputPath 'interaction.json'
if (Test-Path -LiteralPath $reportPath) { throw "An existing report would be overwritten: $reportPath" }
$utf8 = [Text.UTF8Encoding]::new($false)
$logPath = Join-Path $outputPath 'interaction.log'
$hoverSamples = [Collections.Generic.List[object]]::new()
$dragSamples = [Collections.Generic.List[object]]::new()
$report = [ordered]@{ renderer = $Renderer; success = $false; stage = 'launch'; hover = $hoverSamples; drag = $dragSamples }
$process = [Diagnostics.Process]::new()
$started = $false
$leftDown = $false
$previousDpi = [ArkPetsInteraction]::SetThreadDpiAwarenessContext([IntPtr](-4))
$originalCursor = New-Object ArkPetsInteraction+Point
if (-not [ArkPetsInteraction]::GetCursorPos([ref]$originalCursor)) { throw 'Cannot read the current cursor position' }
if (([ArkPetsInteraction]::GetAsyncKeyState(1) -band 0x8000) -ne 0) { throw 'The left mouse button is already held; interaction sample cannot start' }

function Write-InteractionLog([string]$Message) {
    Write-Output $Message
    [IO.File]::AppendAllText($logPath, $Message + [Environment]::NewLine, $utf8)
}

function Assert-SampleRunning {
    if ($process.HasExited) { throw "The sample process exited during $($report.stage) with code $($process.ExitCode)" }
}

function Read-WindowRect([IntPtr]$Window) {
    $rect = New-Object ArkPetsInteraction+Rect
    if (-not [ArkPetsInteraction]::GetWindowRect($Window, [ref]$rect) -or $rect.Right -le $rect.Left -or $rect.Bottom -le $rect.Top) {
        throw "Cannot read a live sample window rectangle: $Window"
    }
    [pscustomobject]@{ x = $rect.Left; y = $rect.Top; width = $rect.Right - $rect.Left; height = $rect.Bottom - $rect.Top }
}

function Show-SampleWindow([IntPtr]$Window, [string]$Name) {
    $visible = [ArkPetsInteraction]::IsWindowVisible($Window)
    Write-InteractionLog "$Name native visibility before show: $visible (HWND $Window)"
    if (-not $visible) { [ArkPetsInteraction]::ShowWindow($Window, 4) | Out-Null }
    if (-not [ArkPetsInteraction]::IsWindowVisible($Window)) { throw "$Name did not become visible" }
    if (-not [ArkPetsInteraction]::SetWindowPos($Window, [IntPtr](-1), 0, 0, 0, 0, 0x13)) { throw "Cannot raise $Name above the host application" }
}

function Wait-SampleWindow([bool]$Pet, [bool]$Present, [int]$TimeoutMs) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-SampleRunning
        $window = [ArkPetsInteraction]::FindWindow([uint32]$process.Id, $Pet)
        if (($window -ne [IntPtr]::Zero) -eq $Present) { return $window }
        Start-Sleep -Milliseconds 25
    } while ($timer.ElapsedMilliseconds -lt $TimeoutMs)
    throw "Timed out waiting for pet=$Pet present=$Present after $TimeoutMs ms"
}

function Client-ScreenPoint([IntPtr]$Window, [double]$X, [double]$Y) {
    $point = New-Object ArkPetsInteraction+Point
    $point.X = [int][Math]::Round($X)
    $point.Y = [int][Math]::Round($Y)
    if (-not [ArkPetsInteraction]::ClientToScreen($Window, [ref]$point)) { throw 'ClientToScreen failed for the sample window' }
    return $point
}

function Move-Pointer([int]$X, [int]$Y) {
    if (-not [ArkPetsInteraction]::SetCursorPos($X, $Y)) { throw "Cannot move the cursor to $X,$Y" }
}

function Assert-PointerTarget([IntPtr]$Window) {
    $point = New-Object ArkPetsInteraction+Point
    if (-not [ArkPetsInteraction]::GetCursorPos([ref]$point)) { throw 'Cannot read the pointer position' }
    $target = [ArkPetsInteraction]::GetAncestor([ArkPetsInteraction]::WindowFromPoint($point), 2)
    if ($target -ne $Window) { throw "Pointer targets HWND $target instead of owned sample HWND $Window; no click was sent" }
}

function Click-DesktopToggle([IntPtr]$Root) {
    Show-SampleWindow $Root 'ROOT'
    $client = New-Object ArkPetsInteraction+Rect
    if (-not [ArkPetsInteraction]::GetClientRect($Root, [ref]$client)) { throw 'Cannot read the ROOT client area' }
    $dpi = [ArkPetsInteraction]::GetDpiForWindow($Root)
    if ($dpi -eq 0) { throw 'GetDpiForWindow failed for ROOT' }
    # toolbar(): 72 logical pixels tall, launch button is the first item in its right-to-left row.
    $point = Client-ScreenPoint $Root ($client.Right - 55 * $dpi / 96.0) (36 * $dpi / 96.0)
    Write-InteractionLog "Click desktop toggle at screen $($point.X),$($point.Y), ROOT DPI $dpi"
    Move-Pointer $point.X $point.Y
    Start-Sleep -Milliseconds 80
    Assert-PointerTarget $Root
    [ArkPetsInteraction]::LeftButton($true)
    try { Start-Sleep -Milliseconds 80 } finally { [ArkPetsInteraction]::LeftButton($false) }
}

function Click-Client([IntPtr]$Window, [double]$X, [double]$Y) {
    $scale = [ArkPetsInteraction]::GetDpiForWindow($Window) / 96.0
    $point = Client-ScreenPoint $Window ($X * $scale) ($Y * $scale)
    Move-Pointer $point.X $point.Y
    Start-Sleep -Milliseconds 80
    Assert-PointerTarget $Window
    [ArkPetsInteraction]::LeftButton($true)
    try { Start-Sleep -Milliseconds 80 } finally { [ArkPetsInteraction]::LeftButton($false) }
    Start-Sleep -Milliseconds 150
}

function Wait-TaskbarStyle([bool]$Visible, [IntPtr]$Previous) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-SampleRunning
        $window = [ArkPetsInteraction]::FindWindow([uint32]$process.Id, $true)
        if ($window -ne [IntPtr]::Zero -and [ArkPetsInteraction]::IsWindowVisible($window)) {
            $style = [ArkPetsInteraction]::GetWindowLong($window, -20)
            $appWindow = ($style -band 0x40000) -ne 0
            $toolWindow = ($style -band 0x80) -ne 0
            if ($appWindow -eq $Visible -and $toolWindow -ne $Visible) {
                if ($window -ne $Previous) { throw 'Taskbar toggle recreated the desktop pet window' }
                return $window
            }
        }
        Start-Sleep -Milliseconds 25
    } while ($timer.ElapsedMilliseconds -lt 5000)
    throw ('Taskbar style did not become visible={0}; old HWND {1}, current HWND {2}, style 0x{3:X}' -f $Visible, $Previous, $window, $style)
}

function Wait-Floor([IntPtr]$Window, [int]$Floor) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-SampleRunning
        $rect = Read-WindowRect $Window
        if ([Math]::Abs($rect.y + $rect.height - $Floor) -le 2) { return }
        Start-Sleep -Milliseconds 50
    } while ($timer.ElapsedMilliseconds -lt 5000)
    throw "Pet bottom $($rect.y + $rect.height) did not reach expected floor $Floor"
}

try {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $executablePath
    $start.Arguments = "--renderer $Renderer"
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
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'Could not launch the interaction sample' }
    $started = $true
    $null = $process.Handle
    $stderr = $process.StandardError.ReadToEndAsync()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $report.process_id = $process.Id
    Write-InteractionLog "Sample PID $($process.Id), renderer $Renderer, reports $outputPath"

    $root = Wait-SampleWindow $false $true 15000
    Show-SampleWindow $root 'ROOT'
    $pet = Wait-SampleWindow $true $true 15000
    Show-SampleWindow $pet 'Desktop pet'
    $dpi = [ArkPetsInteraction]::GetDpiForWindow($pet)
    if ($dpi -eq 0) { throw 'GetDpiForWindow failed for desktop pet' }
    $work = New-Object ArkPetsInteraction+MonitorInfo
    $work.Size = [Runtime.InteropServices.Marshal]::SizeOf($work)
    if (-not [ArkPetsInteraction]::GetMonitorInfo([ArkPetsInteraction]::MonitorFromWindow($pet, 2), [ref]$work)) {
        throw 'Cannot read the desktop pet monitor work area'
    }
    $report.work_area = $work.Work
    $report.pet_dpi = $dpi
    $report.stage = 'settle'
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $stable = 0
    $previous = Read-WindowRect $pet
    do {
        Start-Sleep -Milliseconds 100
        Assert-SampleRunning
        $current = Read-WindowRect $pet
        if ([Math]::Abs($current.x - $previous.x) -le 1 -and [Math]::Abs($current.y - $previous.y) -le 1) { $stable++ } else { $stable = 0 }
        $previous = $current
    } while (($stable -lt 6 -or [Math]::Abs($current.y + $current.height - $work.Work.Bottom) -gt 3) -and $timer.ElapsedMilliseconds -lt 10000)
    if ($stable -lt 6 -or [Math]::Abs($current.y + $current.height - $work.Work.Bottom) -gt 3) { throw 'Desktop pet did not settle at the monitor work-area floor' }
    $initial = $current
    Write-InteractionLog "Settled pet rectangle: $($initial | ConvertTo-Json -Compress)"
    $style = [ArkPetsInteraction]::GetWindowLong($pet, -20)
    if (($style -band 0x80) -eq 0 -or ($style -band 0x40000) -ne 0) {
        throw ('Desktop pet should be hidden from taskbar and Alt+Tab; extended style is 0x{0:X}' -f $style)
    }

    $report.stage = 'hover'
    $client = New-Object ArkPetsInteraction+Rect
    if (-not [ArkPetsInteraction]::GetClientRect($pet, [ref]$client)) { throw 'Cannot read desktop pet client area' }
    $anchor = Client-ScreenPoint $pet ($client.Right / 2.0) ($client.Bottom * 0.6)
    for ($index = 0; $index -lt 24; $index++) {
        if (([ArkPetsInteraction]::GetAsyncKeyState(1) -band 0x8000) -ne 0) { throw 'A mouse button was held during the hover-only sample' }
        $offset = if (($index % 2) -eq 0) { -12 } else { 12 }
        Move-Pointer ($anchor.X + [int]($offset * $dpi / 96.0)) $anchor.Y
        Start-Sleep -Milliseconds 60
        $rect = Read-WindowRect $pet
        $hoverSamples.Add($rect)
        if ([Math]::Abs($rect.x - $initial.x) -gt 2 -or [Math]::Abs($rect.y - $initial.y) -gt 2) {
            throw "Hover moved the desktop pet: initial $($initial.x),$($initial.y), observed $($rect.x),$($rect.y)"
        }
    }
    Write-InteractionLog 'Hover PASS: no button pressed; native pet position stayed within two physical pixels'

    $report.stage = 'drag'
    Move-Pointer $anchor.X $anchor.Y
    Start-Sleep -Milliseconds 80
    Assert-PointerTarget $pet
    [ArkPetsInteraction]::LeftButton($true)
    $leftDown = $true
    Start-Sleep -Milliseconds 100
    for ($step = 1; $step -le 16; $step++) {
        $dx = [int][Math]::Round(80.0 * $step / 16)
        $dy = [int][Math]::Round(-60.0 * $step / 16)
        Move-Pointer ($anchor.X + $dx) ($anchor.Y + $dy)
        Start-Sleep -Milliseconds 40
        $rect = Read-WindowRect $pet
        $dragSamples.Add([pscustomobject]@{ step = $step; cursor_dx = $dx; cursor_dy = $dy; rect = $rect })
        if ($rect.x -lt $work.Work.Left - 2 -or $rect.y -lt $work.Work.Top - 2 -or $rect.x + $rect.width -gt $work.Work.Right + 2 -or $rect.y + $rect.height -gt $work.Work.Bottom + 2) {
            throw 'Desktop pet escaped the monitor work area while dragging'
        }
        if ([Math]::Abs(($rect.x - $initial.x) - $dx) -gt 25 -or [Math]::Abs(($rect.y - $initial.y) - $dy) -gt 25) {
            throw "Drag diverged from the cursor at step ${step}: window displacement $($rect.x - $initial.x),$($rect.y - $initial.y), cursor $dx,$dy"
        }
    }
    Start-Sleep -Milliseconds 250
    $held = Read-WindowRect $pet
    $report.drag_held = $held
    if ([Math]::Abs(($held.x - $initial.x) - 80) -gt 20 -or [Math]::Abs(($held.y - $initial.y) + 60) -gt 20) {
        throw "Held drag did not follow 80,-60 cursor displacement: observed $($held.x - $initial.x),$($held.y - $initial.y)"
    }
    [ArkPetsInteraction]::LeftButton($false)
    $leftDown = $false
    Write-InteractionLog "Drag PASS while held: native displacement $($held.x - $initial.x),$($held.y - $initial.y)"

    $report.stage = 'hide_pet'
    Click-DesktopToggle $root
    $null = Wait-SampleWindow $true $false 2500
    Write-InteractionLog 'ROOT interaction PASS: desktop toggle closed the native pet window'
    $report.stage = 'reopen_pet'
    Click-DesktopToggle $root
    $pet = Wait-SampleWindow $true $true 2500
    $visibleTimer = [Diagnostics.Stopwatch]::StartNew()
    while (-not [ArkPetsInteraction]::IsWindowVisible($pet)) {
        Assert-SampleRunning
        if ($visibleTimer.ElapsedMilliseconds -gt 2500) { throw 'The reopened pet window is hidden' }
        Start-Sleep -Milliseconds 25
    }
    Write-InteractionLog 'ROOT interaction PASS: desktop toggle reopened a visible native pet window'

    if ($DesktopSettings) {
        $report.stage = 'desktop_settings'
        $scale = [ArkPetsInteraction]::GetDpiForWindow($root) / 96.0
        if (-not [ArkPetsInteraction]::SetWindowPos($root, [IntPtr](-1), 10, 10, [int](1200 * $scale), [int](1000 * $scale), 0x10)) { throw 'Cannot resize settings window' }
        Click-Client $root 100 156
        $point = Client-ScreenPoint $root (700 * $scale) (500 * $scale)
        Move-Pointer $point.X $point.Y
        Assert-PointerTarget $root
        [ArkPetsInteraction]::Wheel(-12000)
        Start-Sleep -Milliseconds 500
        $client = New-Object ArkPetsInteraction+Rect
        if (-not [ArkPetsInteraction]::GetClientRect($root, [ref]$client)) { throw 'Cannot read settings client area' }
        $height = $client.Bottom / $scale
        # The settings scroll area is at the bottom; rows are anchored above the footer.
        foreach ($visible in @($true, $false, $true)) {
            Click-Client $root 270 ($height - 192)
            $pet = Wait-TaskbarStyle $visible $pet
            Write-InteractionLog "Taskbar/Alt+Tab style PASS: visible=$visible"
        }
        Click-Client $root 270 ($height - 164)
        foreach ($heightPx in @(120, 0, 80)) {
            Click-Client $root 395 ($height - 130)
            [ArkPetsInteraction]::Key(0x11, $true)
            try {
                [ArkPetsInteraction]::Key(0x41, $true)
                [ArkPetsInteraction]::Key(0x41, $false)
            } finally { [ArkPetsInteraction]::Key(0x11, $false) }
            foreach ($digit in $heightPx.ToString().ToCharArray()) {
                [ArkPetsInteraction]::Key([uint16]$digit, $true)
                [ArkPetsInteraction]::Key([uint16]$digit, $false)
            }
            [ArkPetsInteraction]::Key(0x0D, $true)
            [ArkPetsInteraction]::Key(0x0D, $false)
            Wait-Floor $pet ($work.Monitor.Bottom - $heightPx)
            Write-InteractionLog "Custom floor PASS: $heightPx physical pixels from monitor bottom"
        }
        Click-Client $root 270 ($height - 164)
        Wait-Floor $pet $work.Work.Bottom
        Write-InteractionLog 'Automatic floor PASS: returned to system work area'
    }

    $report.stage = 'close_root'
    $closeTimer = [Diagnostics.Stopwatch]::StartNew()
    if (-not [ArkPetsInteraction]::PostMessage($root, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Could not send WM_CLOSE to the sample ROOT' }
    if (-not $process.WaitForExit(2000)) { throw 'ROOT WM_CLOSE did not terminate the sample within two seconds' }
    $report.close_elapsed_ms = $closeTimer.Elapsed.TotalMilliseconds
    if ($process.ExitCode -ne 0) { throw "Sample process exited with $($process.ExitCode)" }
    $report.stage = 'complete'
    $report.success = $true
    Write-InteractionLog "Interaction PASS ($Renderer): ROOT closed the sample in $([Math]::Round($report.close_elapsed_ms)) ms"
} catch {
    $report.failure = $_.Exception.Message
    throw
} finally {
    if ($leftDown) { [ArkPetsInteraction]::LeftButton($false) }
    [ArkPetsInteraction]::SetCursorPos($originalCursor.X, $originalCursor.Y) | Out-Null
    if ($previousDpi -ne [IntPtr]::Zero) { [ArkPetsInteraction]::SetThreadDpiAwarenessContext($previousDpi) | Out-Null }
    if ($started) {
        if (-not $process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
        [IO.File]::WriteAllText((Join-Path $outputPath 'stderr.log'), $stderr.GetAwaiter().GetResult(), $utf8)
        [IO.File]::WriteAllText((Join-Path $outputPath 'stdout.log'), $stdout.GetAwaiter().GetResult(), $utf8)
    }
    [IO.File]::WriteAllText($reportPath, ($report | ConvertTo-Json -Depth 8), $utf8)
    $process.Dispose()
    Write-Output "Interaction report: $reportPath"
}
