param(
    [string]$Executable = "$PSScriptRoot/../../target/debug/arkpets-ui.exe",
    [ValidateSet('dx12', 'opengl')][string]$Renderer = 'dx12'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class ArkDesktopBackdrop : System.Windows.Forms.Form {
    protected override System.Windows.Forms.CreateParams CreateParams {
        get {
            var attributes = base.CreateParams;
            attributes.ExStyle |= 0x00000008; // WS_EX_TOPMOST, before handle creation.
            return attributes;
        }
    }
}
public static class ArkDesktopTest {
    public delegate bool Visit(IntPtr window, IntPtr context);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(Visit visit, IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder title, int length);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool UpdateWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
}
'@
$previousDpi = [ArkDesktopTest]::SetThreadDpiAwarenessContext([IntPtr](-4))
[ArkDesktopTest]::SetProcessDPIAware() | Out-Null
$captureDirectory = Join-Path ([IO.Path]::GetTempPath()) ('arkpets-compositor-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $captureDirectory | Out-Null
$visibilityLog = Join-Path $captureDirectory 'visibility.log'
$env:ARKPETS_CAPTURE_DIR = $captureDirectory
$exe = (Resolve-Path -LiteralPath $Executable).Path
$repository = (Resolve-Path -LiteralPath "$PSScriptRoot/../..").Path
$process = Start-Process -FilePath $exe -ArgumentList @('--renderer', $Renderer) -WorkingDirectory $repository -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $captureDirectory 'run.log')
$null = $process.Handle
$backdrop = New-Object ArkDesktopBackdrop
$backdrop.FormBorderStyle = 'None'
$backdrop.ShowInTaskbar = $false
$backdrop.StartPosition = 'Manual'
$backdrop.BackColor = [Drawing.Color]::FromArgb(235, 48, 210)
$backdrop.TopMost = $true
$success = $false
$rootVisibilityVerified = $false
$backdropVisibilityLogged = $false
$timer = [Diagnostics.Stopwatch]::StartNew()
$failureLog = Join-Path $captureDirectory 'failure.log'
try {
    while (-not $process.HasExited -and $timer.Elapsed.TotalSeconds -lt 90) {
        $script:petWindow = [IntPtr]::Zero
        $script:rootWindow = [IntPtr]::Zero
        $testPid = $process.Id
        $callback = [ArkDesktopTest+Visit]{ param($window, $context)
            [uint32]$owner = 0
            [ArkDesktopTest]::GetWindowThreadProcessId($window, [ref]$owner) | Out-Null
            if ($owner -eq $testPid) {
                $title = New-Object Text.StringBuilder 256
                [ArkDesktopTest]::GetWindowText($window, $title, 256) | Out-Null
                if ($title.ToString() -eq 'ArkPets') { $script:rootWindow = $window }
                if ($title.ToString().StartsWith('ArkPets - ')) { $script:petWindow = $window }
            }
            return $true
        }
        [ArkDesktopTest]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
        if ($script:rootWindow -ne [IntPtr]::Zero -and -not $rootVisibilityVerified) {
            $visible = [ArkDesktopTest]::IsWindowVisible($script:rootWindow)
            Write-Output "ROOT IsWindowVisible before explicit show: $visible (PID $testPid, HWND $script:rootWindow)" | Tee-Object -FilePath $visibilityLog -Append
            if (-not $visible) {
                [ArkDesktopTest]::ShowWindow($script:rootWindow, 4) | Out-Null
                $visible = [ArkDesktopTest]::IsWindowVisible($script:rootWindow)
                Write-Output "ROOT IsWindowVisible after ShowWindow(SW_SHOWNOACTIVATE): $visible" | Tee-Object -FilePath $visibilityLog -Append
            }
            if (-not $visible) { throw 'The sample ROOT window could not be made visible' }
            $rootVisibilityVerified = $true
        }
        if ($script:petWindow -ne [IntPtr]::Zero -and -not $success -and [ArkDesktopTest]::IsWindowVisible($script:petWindow)) {
            $rect = New-Object ArkDesktopTest+Rect
            if (-not [ArkDesktopTest]::GetWindowRect($script:petWindow, [ref]$rect)) { continue }
            $width = $rect.Right - $rect.Left
            $height = $rect.Bottom - $rect.Top
            if ($width -le 0 -or $height -le 0) { continue }
            if (-not $backdrop.Visible) { $backdrop.Show() }
            $backdropVisible = [ArkDesktopTest]::IsWindowVisible($backdrop.Handle)
            if (-not $backdropVisibilityLogged) {
                Write-Output "Backdrop IsWindowVisible after Form.Show: $backdropVisible (HWND $($backdrop.Handle))" | Tee-Object -FilePath $visibilityLog -Append
                $backdropVisibilityLogged = $true
            }
            if (-not $backdropVisible) {
                [ArkDesktopTest]::ShowWindow($backdrop.Handle, 4) | Out-Null
                $backdropVisible = [ArkDesktopTest]::IsWindowVisible($backdrop.Handle)
                Write-Output "Backdrop IsWindowVisible after ShowWindow(SW_SHOWNOACTIVATE): $backdropVisible" | Tee-Object -FilePath $visibilityLog -Append
            }
            if (-not $backdropVisible) { throw 'The controlled backdrop window could not be made visible' }
            $backdropX = $rect.Left - 20
            $backdropY = $rect.Top - 20
            if (-not [ArkDesktopTest]::SetWindowPos($backdrop.Handle, [IntPtr](-1), $backdropX, $backdropY, $width + 40, $height + 40, 0x10)) { throw 'Cannot position the controlled backdrop' }
            if (-not [ArkDesktopTest]::SetWindowPos($script:petWindow, [IntPtr](-1), 0, 0, 0, 0, 0x13)) { throw 'Cannot place the sample pet above the backdrop' }
            [Windows.Forms.Application]::DoEvents()
            [ArkDesktopTest]::UpdateWindow($backdrop.Handle) | Out-Null
            Start-Sleep -Milliseconds 80
            $backdropRect = New-Object ArkDesktopTest+Rect
            if (-not [ArkDesktopTest]::GetWindowRect($backdrop.Handle, [ref]$backdropRect)) { throw 'Cannot read controlled backdrop rectangle' }
            $after = [ArkDesktopTest]::GetWindow($script:petWindow, 2)
            $point = New-Object ArkDesktopTest+Point
            $point.X = $rect.Left + 3
            $point.Y = $rect.Top + 3
            $under = [ArkDesktopTest]::GetAncestor([ArkDesktopTest]::WindowFromPoint($point), 2)
            $diagnostic = "pet=($($rect.Left),$($rect.Top),$width,$height), backdrop=($($backdropRect.Left),$($backdropRect.Top),$($backdropRect.Right - $backdropRect.Left),$($backdropRect.Bottom - $backdropRect.Top)), dpi=$([ArkDesktopTest]::GetDpiForWindow($backdrop.Handle))/$([ArkDesktopTest]::GetDpiForWindow($script:petWindow)), backdropVisible=$backdropVisible, next=$after, cornerAncestor=$under"
            Write-Output $diagnostic | Tee-Object -FilePath $visibilityLog -Append
            if ($backdropRect.Left -ne $backdropX -or $backdropRect.Top -ne $backdropY) { throw "Controlled backdrop was virtualized or moved: $diagnostic" }
            if ($under -ne $backdrop.Handle -and $under -ne $script:petWindow) {
                $coverTitle = New-Object Text.StringBuilder 256
                [ArkDesktopTest]::GetWindowText($under, $coverTitle, 256) | Out-Null
                throw "Corner obscured by '$coverTitle': $diagnostic"
            }
            $bitmap = [Drawing.Bitmap]::new($width, $height)
            $graphics = [Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, [Drawing.Size]::new($width, $height))
                $corner = $bitmap.GetPixel(3, 3)
                $edgePixels = @(
                    $bitmap.GetPixel(3, 3),
                    $bitmap.GetPixel($width - 4, 3),
                    $bitmap.GetPixel(3, $height - 4),
                    $bitmap.GetPixel($width - 4, $height - 4)
                )
                $observed = Join-Path $captureDirectory 'windows-observed.png'
                if (-not (Test-Path -LiteralPath $observed)) {
                    $bitmap.Save($observed)
                    Write-Output "Observed desktop corner RGB($($corner.R),$($corner.G),$($corner.B)): $observed"
                    $wide = [Drawing.Bitmap]::new($width + 40, $height + 40)
                    $wideGraphics = [Drawing.Graphics]::FromImage($wide)
                    try {
                        $wideGraphics.CopyFromScreen($rect.Left - 20, $rect.Top - 20, 0, 0, [Drawing.Size]::new($width + 40, $height + 40))
                        $wide.Save((Join-Path $captureDirectory 'windows-backdrop.png'))
                    } finally { $wideGraphics.Dispose(); $wide.Dispose() }
                }
                $modelPixels = 0
                for ($y = 25; $y -lt $height - 10; $y += 4) {
                    for ($x = 25; $x -lt $width - 10; $x += 4) {
                        $pixel = $bitmap.GetPixel($x, $y)
                        if ($pixel.ToArgb() -ne $backdrop.BackColor.ToArgb()) { $modelPixels++ }
                    }
                }
                $edgeMatches = @($edgePixels | Where-Object { $_.ToArgb() -eq $backdrop.BackColor.ToArgb() }).Count
                if ($edgeMatches -eq 4 -and $modelPixels -gt 100) {
                    $imagePath = Join-Path $captureDirectory 'windows-composited.png'
                    $bitmap.Save($imagePath)
                    Write-Output "Native compositor PASS: transparent corner reveals RGB(235,48,210); $modelPixels sampled model pixels remain visible."
                    Write-Output $imagePath
                    $success = $true
                    $backdrop.Hide()
                }
            } finally { $graphics.Dispose(); $bitmap.Dispose() }
        }
        Start-Sleep -Milliseconds 30
    }
    if (-not $process.HasExited) { throw 'Desktop verification timed out' }
    $process.WaitForExit()
    Get-Content -LiteralPath (Join-Path $captureDirectory 'run.log') -Encoding UTF8
    if ($process.ExitCode -ne 0) { throw "Application exited with $($process.ExitCode)" }
    if (-not $rootVisibilityVerified) { throw 'The sample ROOT window was never verified visible' }
    if (-not $success) { throw "Windows did not composite the transparent background correctly; inspect $captureDirectory" }
} finally {
    $backdrop.Dispose()
    if (-not $process.HasExited) { $process.Kill() }
    if ($previousDpi -ne [IntPtr]::Zero) { [ArkDesktopTest]::SetThreadDpiAwarenessContext($previousDpi) | Out-Null }
    Remove-Item Env:ARKPETS_CAPTURE_DIR -ErrorAction SilentlyContinue
}
