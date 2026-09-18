param(
    [ValidateSet('debug', 'release')][string]$Configuration = 'release',
    [string]$Executable,
    [string]$OutputDirectory,
    [ValidateRange(8, 120)][int]$DurationSeconds = 8,
    [ValidateRange(2, 60)][int]$WarmupSeconds = 2,
    [ValidateRange(1, 240)][int]$Fps = 60,
    [string]$Model = 'pepe',
    [switch]$RepaintStress,
    [ValidateRange(0, 65536)][int]$MaxPrivateMemoryMiB = 0
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath "$PSScriptRoot/../..").Path
if (-not $Executable) {
    $Executable = Join-Path $repository "target/$Configuration/arkpets-ui.exe"
}
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
if (-not $OutputDirectory) {
    $runName = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $OutputDirectory = Join-Path $repository "target/performance/$runName"
}
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($outputPath) | Out-Null
$framesPath = Join-Path $outputPath 'frames.json'
if (Test-Path -LiteralPath $framesPath) {
    throw "An existing performance report would be overwritten: $framesPath"
}

if (-not ('ArkPetsPerformanceWindow' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class ArkPetsPerformanceWindow {
    public delegate bool Visit(IntPtr window, IntPtr context);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Visit visit, IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder title, int length);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool InvalidateRect(IntPtr window, IntPtr rect, bool erase);
    public static System.Threading.Timer StressRepaints(IntPtr window, uint processId) {
        // Exercise two independently repainting swapchains without moving the user's cursor.
        return new System.Threading.Timer(_ => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == processId) InvalidateRect(window, IntPtr.Zero, false);
        }, null, 0, 5);
    }
    public static IntPtr FindRoot(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, context) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var title = new StringBuilder(256);
            GetWindowText(window, title, title.Capacity);
            if (title.ToString() != "ArkPets") return true;
            found = window;
            return false;
        }, IntPtr.Zero);
        return found;
    }
    public static void CloseOwnedWindows(uint processId) {
        EnumWindows((window, context) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == processId) PostMessage(window, 0x0010, IntPtr.Zero, IntPtr.Zero);
            return true;
        }, IntPtr.Zero);
    }
}
'@
}

$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $executablePath
$start.WorkingDirectory = $repository
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.EnvironmentVariables['ARKPETS_PROFILE_DIR'] = $outputPath
$start.EnvironmentVariables['ARKPETS_PROFILE_MODEL'] = $Model
$start.EnvironmentVariables['ARKPETS_PROFILE_FPS'] = $Fps.ToString()
$start.EnvironmentVariables['ARKPETS_PROFILE_SECONDS'] = $DurationSeconds.ToString()
$start.EnvironmentVariables['ARKPETS_PROFILE_WARMUP'] = $WarmupSeconds.ToString()
$start.EnvironmentVariables.Remove('ARKPETS_CAPTURE_DIR')
$process = [Diagnostics.Process]::new()
$process.StartInfo = $start
$samples = [Collections.Generic.List[object]]::new()
$started = $false
$utf8 = [Text.UTF8Encoding]::new($false)
$visibilityLog = Join-Path $outputPath 'visibility.log'
$rootVisibilityLogged = $false
$repaintTimer = $null

try {
    if (-not $process.Start()) { throw 'Could not start the performance sample process' }
    $started = $true
    $null = $process.Handle
    $standardError = $process.StandardError.ReadToEndAsync()
    $standardOutput = $process.StandardOutput.ReadToEndAsync()
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $timeoutSeconds = $WarmupSeconds + $DurationSeconds + 30
    while (-not $process.HasExited -and $timer.Elapsed.TotalSeconds -lt $timeoutSeconds) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $rootWindow = [ArkPetsPerformanceWindow]::FindRoot([uint32]$process.Id)
            $rootVisible = $false
            if ($rootWindow -ne [IntPtr]::Zero) {
                $rootVisible = [ArkPetsPerformanceWindow]::IsWindowVisible($rootWindow)
                if (-not $rootVisibilityLogged -or -not $rootVisible) {
                    $message = "ROOT IsWindowVisible before explicit show: $rootVisible (PID $($process.Id), HWND $rootWindow)"
                    Write-Output $message
                    [IO.File]::AppendAllText($visibilityLog, $message + [Environment]::NewLine, $utf8)
                    $rootVisibilityLogged = $true
                }
                if (-not $rootVisible) {
                    [ArkPetsPerformanceWindow]::ShowWindow($rootWindow, 4) | Out-Null
                    $rootVisible = [ArkPetsPerformanceWindow]::IsWindowVisible($rootWindow)
                    $message = "ROOT IsWindowVisible after ShowWindow(SW_SHOWNOACTIVATE): $rootVisible"
                    Write-Output $message
                    [IO.File]::AppendAllText($visibilityLog, $message + [Environment]::NewLine, $utf8)
                    if (-not $rootVisible) { throw 'The sample settings window could not be made visible' }
                }
                if ($RepaintStress -and $null -eq $repaintTimer) {
                    $repaintTimer = [ArkPetsPerformanceWindow]::StressRepaints($rootWindow, [uint32]$process.Id)
                }
            }
            $samples.Add([pscustomobject]@{
                unix_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                working_set_bytes = $process.WorkingSet64
                private_bytes = $process.PrivateMemorySize64
                root_visible = $rootVisible
            })
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $process.HasExited) {
        $process.Kill()
        $process.WaitForExit()
        [IO.File]::WriteAllText((Join-Path $outputPath 'stderr.log'), $standardError.GetAwaiter().GetResult(), $utf8)
        throw "Performance sample timed out after $timeoutSeconds seconds (PID $($process.Id))"
    }
    $process.WaitForExit()
    $errorText = $standardError.GetAwaiter().GetResult()
    $outputText = $standardOutput.GetAwaiter().GetResult()
    [IO.File]::WriteAllText((Join-Path $outputPath 'stderr.log'), $errorText, $utf8)
    [IO.File]::WriteAllText((Join-Path $outputPath 'stdout.log'), $outputText, $utf8)
    if ($process.ExitCode -ne 0) {
        throw "Application exited with $($process.ExitCode). $errorText"
    }
    if (-not (Test-Path -LiteralPath $framesPath)) {
        throw 'The application did not produce frames.json; build the version with performance instrumentation first'
    }
    $frames = Get-Content -LiteralPath $framesPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($frames.process_id -ne $process.Id) {
        throw 'Frame report process ID does not match the process started by this sample'
    }
    $measured = @($samples | Where-Object {
        $_.unix_ms -ge $frames.measurement_start_unix_ms -and $_.unix_ms -le $frames.measurement_end_unix_ms
    })
    if ($measured.Count -lt 10) { throw 'Too few memory samples inside the actual frame measurement window' }
    if ($measured | Where-Object { -not $_.root_visible }) {
        throw 'The settings window was not visible during every measured sample'
    }
    $workingSet = $measured | Measure-Object -Property working_set_bytes -Average -Maximum -Minimum
    $privateBytes = $measured | Measure-Object -Property private_bytes -Average -Maximum -Minimum
    $report = [ordered]@{
        version = 1
        process_id = $process.Id
        executable = $executablePath
        executable_sha256 = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash
        sample_interval_ms = 100
        sample_count = $measured.Count
        root_visibility_verified = $true
        root_repaint_stress = [bool]$RepaintStress
        measurement_start_unix_ms = $frames.measurement_start_unix_ms
        measurement_end_unix_ms = $frames.measurement_end_unix_ms
        working_set_bytes = [ordered]@{ mean = $workingSet.Average; min = $workingSet.Minimum; max = $workingSet.Maximum }
        private_bytes = [ordered]@{ mean = $privateBytes.Average; min = $privateBytes.Minimum; max = $privateBytes.Maximum }
        samples = $measured
    }
    [IO.File]::WriteAllText((Join-Path $outputPath 'memory.json'), ($report | ConvertTo-Json -Depth 8), $utf8)
    Write-Output "Reports: $outputPath"
    Write-Output ('Model {0}, limit {1} FPS, build debug_assertions={2}' -f $frames.settings.model, $frames.settings.fps, $frames.debug_assertions)
    Write-Output ('Desktop callback rate {0:N2} Hz; interval p50/p95/p99 {1:N2}/{2:N2}/{3:N2} ms (not display-present FPS)' -f $frames.desktop.callback_rate_hz, $frames.desktop.callback_interval_ms.p50, $frames.desktop.callback_interval_ms.p95, $frames.desktop.callback_interval_ms.p99)
    Write-Output ('Working set mean/peak {0:N1}/{1:N1} MiB; private bytes mean/peak {2:N1}/{3:N1} MiB' -f ($workingSet.Average / 1MB), ($workingSet.Maximum / 1MB), ($privateBytes.Average / 1MB), ($privateBytes.Maximum / 1MB))
    if ($RepaintStress -and $frames.root.callback_interval_ms.samples -lt $DurationSeconds * 10) {
        throw 'Repaint stress did not produce enough ROOT callbacks; the GPU regression was not exercised'
    }
    if ($MaxPrivateMemoryMiB -gt 0 -and $privateBytes.Maximum -gt $MaxPrivateMemoryMiB * 1MB) {
        throw "Private memory exceeded the $MaxPrivateMemoryMiB MiB regression limit; see $outputPath"
    }
} finally {
    if ($null -ne $repaintTimer) { $repaintTimer.Dispose() }
    if ($started -and -not $process.HasExited) {
        [ArkPetsPerformanceWindow]::CloseOwnedWindows([uint32]$process.Id)
        if (-not $process.WaitForExit(5000)) {
            Write-Warning "Sample process PID $($process.Id) ignored normal window close; terminating only this sample process"
            $process.Kill()
            $process.WaitForExit()
        }
    }
    $process.Dispose()
}
