param(
    [string]$PluginExecutable = "$env:APPDATA\com.agiletiles.app\user_plugins\screenshot\agile-tiles-screenshot-core.exe",
    [ValidateRange(1, 64)]
    [int]$ScrollSteps = 8,
    [ValidateRange(1, 120)]
    [int]$LinesPerStep = 12
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $PluginExecutable -PathType Leaf)) {
    throw "screenshot plugin executable not found: $PluginExecutable"
}

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class ScreenshotRealScrollNative {
    public sealed class WindowInfo {
        public IntPtr Handle;
        public int Left;
        public int Top;
        public int Width;
        public int Height;
    }

    private struct RECT {
        public int Left, Top, Right, Bottom;
    }

    private struct POINT {
        public int X, Y;
    }

    private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lparam);
    private delegate bool EnumChildWindowsProc(IntPtr hwnd, IntPtr lparam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lparam);

    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumChildWindowsProc callback, IntPtr lparam);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    private static extern IntPtr GetDC(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);

    [DllImport("gdi32.dll")]
    private static extern uint GetPixel(IntPtr dc, int x, int y);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr hwnd, StringBuilder className, int maxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr hwnd, StringBuilder windowText, int maxCount);

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);

    [DllImport("user32.dll")]
    private static extern bool GetClientRect(IntPtr hwnd, out RECT rect);

    [DllImport("user32.dll")]
    private static extern bool ClientToScreen(IntPtr hwnd, ref POINT point);

    [DllImport("user32.dll", EntryPoint = "SetWindowPos")]
    private static extern bool SetWindowPos(
        IntPtr hwnd,
        IntPtr insertAfter,
        int x,
        int y,
        int width,
        int height,
        uint flags);

    [DllImport("user32.dll")]
    private static extern bool ShowWindow(IntPtr hwnd, int command);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern IntPtr SetFocus(IntPtr hwnd);

    [DllImport("user32.dll", EntryPoint = "SendMessageW")]
    private static extern IntPtr SendMessage(
        IntPtr hwnd,
        uint message,
        UIntPtr wparam,
        IntPtr lparam);

    [DllImport("user32.dll")]
    private static extern bool InvalidateRect(IntPtr hwnd, IntPtr rect, bool erase);

    [DllImport("user32.dll")]
    private static extern bool UpdateWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr context);

    public static WindowInfo[] FindClassWindows(uint processId, string className) {
        var result = new List<WindowInfo>();
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || !IsWindowVisible(hwnd)) return true;
            var name = new StringBuilder(128);
            if (GetClassName(hwnd, name, name.Capacity) == 0 ||
                name.ToString() != className) return true;
            RECT rect;
            if (!GetWindowRect(hwnd, out rect)) return true;
            result.Add(new WindowInfo {
                Handle = hwnd,
                Left = rect.Left,
                Top = rect.Top,
                Width = rect.Right - rect.Left,
                Height = rect.Bottom - rect.Top
            });
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }

    public static IntPtr FindTopLevel(uint processId) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner == processId && IsWindowVisible(hwnd)) {
                result = hwnd;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return result;
    }

    public static IntPtr FindWindowByTitle(uint processId, string titleFragment) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || !IsWindowVisible(hwnd)) return true;
            var title = new StringBuilder(512);
            GetWindowText(hwnd, title, title.Capacity);
            if (title.ToString().IndexOf(titleFragment, StringComparison.OrdinalIgnoreCase) < 0) return true;
            result = hwnd;
            return false;
        }, IntPtr.Zero);
        return result;
    }

    public static IntPtr FindEdit(uint processId) {
        IntPtr richEdit = IntPtr.Zero;
        IntPtr plainEdit = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || !IsWindowVisible(hwnd)) return true;
            EnumChildWindows(hwnd, (child, __) => {
                var name = new StringBuilder(128);
                GetClassName(child, name, name.Capacity);
                var value = name.ToString();
                if (value.IndexOf("RichEdit", StringComparison.OrdinalIgnoreCase) >= 0) {
                    richEdit = child;
                } else if (value == "Edit") {
                    plainEdit = child;
                }
                return true;
            }, IntPtr.Zero);
            return true;
        }, IntPtr.Zero);
        return richEdit != IntPtr.Zero ? richEdit : plainEdit;
    }

    public static WindowInfo GetClientScreenRect(IntPtr hwnd) {
        RECT client;
        POINT origin;
        if (!GetClientRect(hwnd, out client)) {
            return new WindowInfo();
        }
        origin = new POINT { X = 0, Y = 0 };
        if (!ClientToScreen(hwnd, ref origin)) {
            return new WindowInfo();
        }
        return new WindowInfo {
            Handle = hwnd,
            Left = origin.X,
            Top = origin.Y,
            Width = client.Right - client.Left,
            Height = client.Bottom - client.Top
        };
    }

    public static bool MoveWindow(IntPtr hwnd, int x, int y, int width, int height) {
        return SetWindowPos(hwnd, IntPtr.Zero, x, y, width, height, 0x0040 | 0x0010);
    }

    public static void Activate(IntPtr hwnd) {
        ShowWindow(hwnd, 9);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }

    public static void SetText(IntPtr hwnd, string value) {
        var pointer = Marshal.StringToHGlobalUni(value);
        try {
            SendMessage(hwnd, 0x000c, UIntPtr.Zero, pointer);
        } finally {
            Marshal.FreeHGlobal(pointer);
        }
    }

    public static int GetFirstVisibleLine(IntPtr hwnd) {
        return SendMessage(hwnd, 0x00ce, UIntPtr.Zero, IntPtr.Zero).ToInt32();
    }

    public static void ScrollLines(IntPtr hwnd, int lines) {
        Activate(hwnd);
        for (var index = 0; index < lines; index++) {
            SendMessage(hwnd, 0x00b6, UIntPtr.Zero, new IntPtr(1));
        }
        InvalidateRect(hwnd, IntPtr.Zero, false);
        UpdateWindow(hwnd);
    }

    public static int ForegroundProcessId() {
        var hwnd = GetForegroundWindow();
        uint processId;
        GetWindowThreadProcessId(hwnd, out processId);
        return (int)processId;
    }

    public static byte[] GetProbe(int left, int top, int width, int height) {
        const int grid = 24;
        var result = new byte[grid * grid];
        var dc = GetDC(IntPtr.Zero);
        if (dc == IntPtr.Zero) return result;
        try {
            var index = 0;
            for (var gy = 0; gy < grid; gy++) {
                var y = ((gy * 2 + 1) * height / (grid * 2));
                for (var gx = 0; gx < grid; gx++) {
                    var x = ((gx * 2 + 1) * width / (grid * 2));
                    var color = GetPixel(dc, left + x, top + y);
                    var red = color & 0xff;
                    var green = (color >> 8) & 0xff;
                    var blue = (color >> 16) & 0xff;
                    result[index++] = (byte)((red * 77 + green * 150 + blue * 29) / 256);
                }
            }
        } finally {
            ReleaseDC(IntPtr.Zero, dc);
        }
        return result;
    }

    public static IntPtr PackPoint(int x, int y) {
        return (IntPtr)((y << 16) | (x & 0xffff));
    }

    public static IntPtr SendWindowMessage(
        IntPtr hwnd,
        uint message,
        UIntPtr wparam,
        IntPtr lparam) {
        return SendMessage(hwnd, message, wparam, lparam);
    }
}
"@

[ScreenshotRealScrollNative]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

$foregroundProcessId = [ScreenshotRealScrollNative]::ForegroundProcessId()
if ($foregroundProcessId -eq 0) {
    throw "interactive desktop has no foreground window; unlock Windows and make an app window visible before running real-scroll smoke"
}
$foregroundProcess = Get-Process -Id $foregroundProcessId -ErrorAction SilentlyContinue
if ($foregroundProcess -and $foregroundProcess.ProcessName -eq "LockApp") {
    throw "interactive desktop is locked (LockApp is foreground); unlock Windows before running real-scroll smoke"
}

$pipeName = "agiletiles-screenshot-real-scroll-$([guid]::NewGuid().ToString('N'))"
$pipePath = "\\.\pipe\$pipeName"
$errorPath = Join-Path ([IO.Path]::GetTempPath()) "$pipeName.err"
$server = [System.IO.Pipes.NamedPipeServerStream]::new(
    $pipeName,
    [System.IO.Pipes.PipeDirection]::InOut,
    1,
    [System.IO.Pipes.PipeTransmissionMode]::Byte,
    [System.IO.Pipes.PipeOptions]::Asynchronous
)
$plugin = $null
$notepad = $null
$fixturePath = $null

function Send-Frame([string]$Json) {
    $payload = [Text.Encoding]::UTF8.GetBytes($Json)
    $header = [BitConverter]::GetBytes([uint32]$payload.Length)
    $server.Write($header, 0, $header.Length)
    $server.Write($payload, 0, $payload.Length)
    $server.Flush()
    $responseHeader = New-Object byte[] 4
    if ($server.Read($responseHeader, 0, 4) -ne 4) {
        throw "IPC response header read failed"
    }
    $length = [BitConverter]::ToUInt32($responseHeader, 0)
    $response = New-Object byte[] $length
    $offset = 0
    while ($offset -lt $length) {
        $read = $server.Read($response, $offset, $length - $offset)
        if ($read -le 0) {
            throw "IPC response body read failed"
        }
        $offset += $read
    }
    [Text.Encoding]::UTF8.GetString($response, 0, $offset)
}

function Get-PluginWindows {
    @([ScreenshotRealScrollNative]::FindClassWindows(
        [uint32]$plugin.Id,
        "AgileTilesScreenshotCoreV2"))
}

function Send-NativeMessage(
    [IntPtr]$Handle,
    [uint32]$Message,
    [uint64]$WParam,
    [int]$X = 0,
    [int]$Y = 0
) {
    [ScreenshotRealScrollNative]::SendWindowMessage(
        $Handle,
        $Message,
        [UIntPtr]::new($WParam),
        [ScreenshotRealScrollNative]::PackPoint($X, $Y)) | Out-Null
}

try {
    $rows = 1..640 | ForEach-Object {
        "ROW-$($_.ToString('D4'))  The quick brown fox jumps over a deterministic content row $((($_ * 37) % 997).ToString('D3'))"
    }
    $fixturePath = Join-Path ([IO.Path]::GetTempPath()) "agiletiles-real-scroll-$([guid]::NewGuid().ToString('N')).txt"
    [IO.File]::WriteAllText($fixturePath, ($rows -join [Environment]::NewLine), [Text.Encoding]::UTF8)
    $fixtureName = [IO.Path]::GetFileName($fixturePath)
    $notepad = Start-Process -FilePath "$env:SystemRoot\System32\notepad.exe" -ArgumentList @($fixturePath) -PassThru -WindowStyle Normal
    $notepadHandle = [IntPtr]::Zero
    for ($attempt = 0; $attempt -lt 40 -and $notepadHandle -eq [IntPtr]::Zero; $attempt++) {
        Start-Sleep -Milliseconds 200
        $notepad.Refresh()
        $notepadHandle = [ScreenshotRealScrollNative]::FindWindowByTitle([uint32]$notepad.Id, $fixtureName)
    }
    if ($notepadHandle -eq [IntPtr]::Zero) {
        throw "visible Notepad document window was not found for process $($notepad.Id); confirm the desktop is unlocked and interactive"
    }

    if (-not [ScreenshotRealScrollNative]::MoveWindow($notepadHandle, 120, 100, 1100, 760)) {
        throw "Notepad window could not be positioned"
    }
    $edit = [ScreenshotRealScrollNative]::FindEdit([uint32]$notepad.Id)
    if ($edit -eq [IntPtr]::Zero) {
        throw "Notepad edit control was not found"
    }
    Start-Sleep -Milliseconds 700
    $client = [ScreenshotRealScrollNative]::GetClientScreenRect($edit)
    if ($client.Width -lt 720 -or $client.Height -lt 400) {
        throw "Notepad edit client is too small: $($client.Width)x$($client.Height)"
    }
    $foregroundProcess = Get-Process -Id ([ScreenshotRealScrollNative]::ForegroundProcessId()) -ErrorAction SilentlyContinue
    if ($foregroundProcess -and $foregroundProcess.ProcessName -eq "LockApp") {
        throw "interactive desktop is locked (LockApp is foreground); unlock Windows before running real-scroll smoke"
    }

    $plugin = Start-Process -FilePath $PluginExecutable -ArgumentList @("--pipe", $pipePath, "--plugin-id", "screenshot") -PassThru -WindowStyle Hidden -RedirectStandardError $errorPath
    $wait = $server.BeginWaitForConnection($null, $null)
    if (-not $wait.AsyncWaitHandle.WaitOne(5000)) {
        throw "IPC connection timed out"
    }
    $server.EndWaitForConnection($wait)

    $handshake = Send-Frame '{"type":"handshake","payload":{"requestId":"real-1","pluginId":"screenshot","apiVersion":1,"hostVersion":"real-scroll-smoke","capabilities":[]}}'
    if ($handshake -notmatch '"accepted":true') {
        throw "plugin handshake was not accepted"
    }
    $start = Send-Frame '{"type":"request","payload":{"requestId":"real-2","pluginId":"screenshot","kind":"call","method":"start_capture","payload":{"mode":"scroll"}}}'
    if ($start -notmatch '"ok":true') {
        throw "scroll start request failed: $start"
    }
    Start-Sleep -Milliseconds 900

    $overlay = Get-PluginWindows | Sort-Object { $_.Width * $_.Height } -Descending | Select-Object -First 1
    if ($null -eq $overlay -or $overlay.Width -lt 400 -or $overlay.Height -lt 300) {
        throw "capture overlay was not found"
    }

    $left = $client.Left + 24
    $top = $client.Top + 24
    $selectionWidth = [Math]::Min(720, $client.Width - 48)
    $selectionHeight = [Math]::Min(400, $client.Height - 48)
    $localLeft = $left - $overlay.Left
    $localTop = $top - $overlay.Top
    $localRight = $localLeft + $selectionWidth
    $localBottom = $localTop + $selectionHeight
    Send-NativeMessage $overlay.Handle 0x0201 1 $localLeft $localTop
    Send-NativeMessage $overlay.Handle 0x0200 1 $localRight $localBottom
    Send-NativeMessage $overlay.Handle 0x0202 0 $localRight $localBottom
    Start-Sleep -Milliseconds 1000

    $scrollWindows = Get-PluginWindows
    $toolbar = $scrollWindows |
        Where-Object { $_.Width -gt 250 -and $_.Height -lt 200 } |
        Select-Object -First 1
    if ($null -eq $toolbar) {
        throw "scroll toolbar was not created"
    }
    if ($ScrollSteps -lt 1 -or $LinesPerStep -lt 1) {
        throw "ScrollSteps and LinesPerStep must both be positive"
    }
    $beforeLine = [ScreenshotRealScrollNative]::GetFirstVisibleLine($edit)
    $lastLine = $beforeLine
    $probeDiff = 0
    $probeDiffs = @()
    $peakWorkingSet = 0L
    $captureStarted = [Diagnostics.Stopwatch]::StartNew()
    for ($step = 1; $step -le $ScrollSteps; $step++) {
        [ScreenshotRealScrollNative]::Activate($notepadHandle)
        $beforeProbe = [ScreenshotRealScrollNative]::GetProbe($left, $top, $selectionWidth, $selectionHeight)
        [ScreenshotRealScrollNative]::ScrollLines($edit, $LinesPerStep)
        Start-Sleep -Milliseconds 700
        $afterProbe = [ScreenshotRealScrollNative]::GetProbe($left, $top, $selectionWidth, $selectionHeight)
        $stepProbeDiff = 0
        for ($index = 0; $index -lt $beforeProbe.Length; $index++) {
            if ([Math]::Abs([int]$beforeProbe[$index] - [int]$afterProbe[$index]) -ge 7) {
                $stepProbeDiff++
            }
        }
        if ($stepProbeDiff -eq 0) {
            throw "screen probe did not detect step " + $step + " content change"
        }
        $probeDiff += $stepProbeDiff
        $probeDiffs += $stepProbeDiff
        $afterLine = [ScreenshotRealScrollNative]::GetFirstVisibleLine($edit)
        if ($afterLine -le $lastLine) {
            throw "Notepad content did not scroll at step " + $step + ": before=" + $lastLine + " after=" + $afterLine
        }
        $lastLine = $afterLine

        $processSnapshot = Get-Process -Id $plugin.Id -ErrorAction SilentlyContinue
        if (-not $processSnapshot) {
            throw "screenshot plugin exited during real scroll step " + $step
        }
        $peakWorkingSet = [Math]::Max([int64]$peakWorkingSet, [int64]$processSnapshot.WorkingSet64)

        $statusRequest = '{"type":"request","payload":{"requestId":"real-step-' + $step + '","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}'
        $stepStatus = (Send-Frame $statusRequest | ConvertFrom-Json).payload.result
        if (-not $stepStatus.active) {
            throw "real scroll session ended during step " + $step
        }
        if ($stepStatus.error) {
            throw "real scroll session reported an error at step " + $step + ": " + $stepStatus.error
        }
    }
    $captureElapsedMs = [int]$captureStarted.ElapsedMilliseconds
    $afterLine = $lastLine

    Start-Sleep -Milliseconds 600
    $active = (Send-Frame '{"type":"request","payload":{"requestId":"real-3","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}' | ConvertFrom-Json).payload.result
    if (-not $active.active) {
        throw "real scroll session ended unexpectedly"
    }
    if ($active.error) {
        throw "real scroll session reported an error: $($active.error); line shift=$($afterLine - $beforeLine), probe diff=$probeDiff, foreground pid=$([ScreenshotRealScrollNative]::ForegroundProcessId()), notepad pid=$($notepad.Id)"
    }

    Send-NativeMessage $toolbar.Handle 0x0201 1 ((20 * 34) + 17) 23
    Send-NativeMessage $toolbar.Handle 0x0202 0 ((20 * 34) + 17) 23
    Start-Sleep -Milliseconds 900
    $finished = (Send-Frame '{"type":"request","payload":{"requestId":"real-4","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}' | ConvertFrom-Json).payload.result
    if ($finished.active) {
        throw "real scroll finish did not close the session"
    }
    if ($null -eq $finished.lastSize -or
        $finished.lastSize.width -ne $selectionWidth -or
        $finished.lastSize.height -le $selectionHeight) {
        throw "real scroll did not append content (line shift=$($afterLine - $beforeLine), client=$($client.Width)x$($client.Height), selection=$($selectionWidth)x$($selectionHeight)): $($finished | ConvertTo-Json -Compress)"
    }

    $shutdown = Send-Frame '{"type":"request","payload":{"requestId":"real-5","pluginId":"screenshot","kind":"shutdown","payload":null}}'
    if ($shutdown -notmatch '"ok":true') {
        throw "plugin shutdown failed: $shutdown"
    }
    Start-Sleep -Milliseconds 700
    if (Get-Process -Id $plugin.Id -ErrorAction SilentlyContinue) {
        throw "plugin did not exit naturally"
    }
    Write-Output "screenshot-plugin-real-scroll-smoke=passed"
    Write-Output "scroll-steps=$ScrollSteps"
    Write-Output "lines-per-step=$LinesPerStep"
    Write-Output "notepad-line-shift=$($afterLine - $beforeLine)"
    Write-Output "screen-probe-diff=$probeDiff"
    Write-Output "probe-diffs=$($probeDiffs -join ",")"
    Write-Output "capture-elapsed-ms=$captureElapsedMs"
    Write-Output "plugin-peak-working-set-bytes=$peakWorkingSet"
    Write-Output "assembled-size=$($finished.lastSize.width)x$($finished.lastSize.height)"
}
finally {
    if ($plugin) {
        Stop-Process -Id $plugin.Id -Force -ErrorAction SilentlyContinue
    }
    if ($notepad) {
        try {
            $notepad.CloseMainWindow() | Out-Null
            Start-Sleep -Milliseconds 500
        } catch {
            Write-Output "notepad-close-warning=$($_.Exception.Message)"
        }
    }
    if ($fixturePath -and (Test-Path -LiteralPath $fixturePath)) {
        Remove-Item -LiteralPath $fixturePath -Force -ErrorAction SilentlyContinue
    }
    $server.Dispose()
    if (Test-Path -LiteralPath $errorPath) {
        $stderr = Get-Content -LiteralPath $errorPath -Raw
        if ($stderr -and $stderr.Trim()) {
            Write-Output "plugin-stderr=$stderr"
        }
        Remove-Item -LiteralPath $errorPath -Force -ErrorAction SilentlyContinue
    }
}
