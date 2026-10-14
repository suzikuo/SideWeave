param(
    [string]$PluginExecutable = "$env:APPDATA\com.agiletiles.app\user_plugins\screenshot\agile-tiles-screenshot-core.exe"
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

public static class ScreenshotNativeSmoke {
    public sealed class WindowInfo {
        public IntPtr Handle;
        public int Left;
        public int Top;
        public int Width;
        public int Height;
    }

    private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lparam);
    private delegate bool EnumChildWindowsProc(IntPtr hwnd, IntPtr lparam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lparam);

    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumChildWindowsProc callback, IntPtr lparam);


    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr hwnd, StringBuilder className, int maxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int maxCount);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hwnd);


    [StructLayout(LayoutKind.Sequential)]
    private struct RECT { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);

    [DllImport("user32.dll", EntryPoint="SendMessageW")]
    public static extern IntPtr SendMessage(IntPtr hwnd, uint message, UIntPtr wparam, IntPtr lparam);

    public static void SendMessageAsync(IntPtr hwnd, uint message, UIntPtr wparam, IntPtr lparam) {
        var thread = new System.Threading.Thread(new System.Threading.ThreadStart(delegate {
            SendMessage(hwnd, message, wparam, lparam);
        }));
        thread.IsBackground = true;
        thread.Start();
    }

    [DllImport("user32.dll")]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr context);

    public static WindowInfo[] FindVisible(uint processId) {
        var result = new List<WindowInfo>();
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || !IsWindowVisible(hwnd)) return true;
            var name = new StringBuilder(128);
            if (GetClassName(hwnd, name, name.Capacity) == 0 ||
                name.ToString() != "AgileTilesScreenshotCoreV2") return true;
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

    public static IntPtr FindAffirmativeButton(IntPtr dialog) {
        IntPtr result = IntPtr.Zero;
        EnumChildWindows(dialog, (hwnd, _) => {
            var name = new StringBuilder(128);
            var title = new StringBuilder(256);
            GetClassName(hwnd, name, name.Capacity);
            GetWindowText(hwnd, title, title.Capacity);
            if (name.ToString() == "Button" &&
                (title.ToString().IndexOf("\u662f", StringComparison.OrdinalIgnoreCase) >= 0 ||
                 title.ToString().IndexOf("Yes", StringComparison.OrdinalIgnoreCase) >= 0 ||
                 title.ToString().IndexOf("OK", StringComparison.OrdinalIgnoreCase) >= 0 ||
                 title.ToString().IndexOf("\u4fdd\u5b58", StringComparison.OrdinalIgnoreCase) >= 0 ||
                 title.ToString().IndexOf("\u786e\u5b9a", StringComparison.OrdinalIgnoreCase) >= 0)) {
                result = hwnd;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return result;
    }

    public static IntPtr[] FindDialogs(uint processId) {
        var result = new List<IntPtr>();
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || !IsWindowVisible(hwnd)) return true;
            var name = new StringBuilder(128);
            if (GetClassName(hwnd, name, name.Capacity) > 0 &&
                name.ToString() == "#32770") result.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }

    public static IntPtr PackPoint(int x, int y) {
        return (IntPtr)((y << 16) | (x & 0xffff));
    }
}
"@

[ScreenshotNativeSmoke]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

$pipeName = "agiletiles-screenshot-native-ui-$([guid]::NewGuid().ToString('N'))"
$pipePath = "\\.\pipe\$pipeName"
$errorPath = Join-Path ([IO.Path]::GetTempPath()) "$pipeName.err"
$server = [System.IO.Pipes.NamedPipeServerStream]::new(
    $pipeName,
    [System.IO.Pipes.PipeDirection]::InOut,
    1,
    [System.IO.Pipes.PipeTransmissionMode]::Byte,
    [System.IO.Pipes.PipeOptions]::Asynchronous
)
$process = $null

function Send-Frame([string]$Json) {
    $payload = [Text.Encoding]::UTF8.GetBytes($Json)
    $header = [BitConverter]::GetBytes([uint32]$payload.Length)
    $server.Write($header, 0, $header.Length)
    $server.Write($payload, 0, $payload.Length)
    $server.Flush()
    $responseHeader = New-Object byte[] 4
    if ($server.Read($responseHeader, 0, 4) -ne 4) { throw "IPC response header read failed" }
    $length = [BitConverter]::ToUInt32($responseHeader, 0)
    $response = New-Object byte[] $length
    $offset = 0
    while ($offset -lt $length) {
        $read = $server.Read($response, $offset, $length - $offset)
        if ($read -le 0) { throw "IPC response body read failed" }
        $offset += $read
    }
    [Text.Encoding]::UTF8.GetString($response, 0, $offset)
}

function Get-VisibleNativeWindows {
    @([ScreenshotNativeSmoke]::FindVisible([uint32]$process.Id))
}

function Send-NativeHandleMessage([IntPtr]$Handle, [uint32]$Message, [uint64]$WParam) {
    [ScreenshotNativeSmoke]::SendMessage(
        $Handle,
        $Message,
        [UIntPtr]::new($WParam),
        [IntPtr]::Zero) | Out-Null
}

function Send-NativeHandleMessageAsync([IntPtr]$Handle, [uint32]$Message, [uint64]$WParam) {
    [ScreenshotNativeSmoke]::SendMessageAsync(
        $Handle,
        $Message,
        [UIntPtr]::new($WParam),
        [IntPtr]::Zero)
}

function Send-NativeMessage($Window, [uint32]$Message, [uint64]$WParam, [int]$X = 0, [int]$Y = 0) {
    [ScreenshotNativeSmoke]::SendMessage(
        $Window.Handle,
        $Message,
        [UIntPtr]::new($WParam),
        [ScreenshotNativeSmoke]::PackPoint($X, $Y)) | Out-Null
}

function Send-NativeMessageAsync($Window, [uint32]$Message, [uint64]$WParam, [int]$X = 0, [int]$Y = 0) {
    [ScreenshotNativeSmoke]::SendMessageAsync(
        $Window.Handle,
        $Message,
        [UIntPtr]::new($WParam),
        [ScreenshotNativeSmoke]::PackPoint($X, $Y))
}

try {
    $process = Start-Process -FilePath $PluginExecutable `
        -ArgumentList @("--pipe", $pipePath, "--plugin-id", "screenshot") `
        -PassThru -WindowStyle Hidden -RedirectStandardError $errorPath
    $wait = $server.BeginWaitForConnection($null, $null)
    if (-not $wait.AsyncWaitHandle.WaitOne(5000)) { throw "IPC connection timed out" }
    $server.EndWaitForConnection($wait)

    $handshake = Send-Frame '{"type":"handshake","payload":{"requestId":"native-1","pluginId":"screenshot","apiVersion":1,"hostVersion":"native-ui-smoke","capabilities":[]}}'
    if ($handshake -notmatch '"accepted":true') { throw "plugin handshake was not accepted" }
    $start = Send-Frame '{"type":"request","payload":{"requestId":"native-2","pluginId":"screenshot","kind":"call","method":"start_capture","payload":{"mode":"scroll"}}}'
    if ($start -notmatch '"ok":true') { throw "scroll start request failed: $start" }
    Start-Sleep -Milliseconds 900

    $initialWindows = Get-VisibleNativeWindows
    $overlay = $initialWindows | Sort-Object { $_.Width * $_.Height } -Descending | Select-Object -First 1
    if ($null -eq $overlay -or $overlay.Width -lt 200 -or $overlay.Height -lt 200) {
        throw "native capture overlay was not found"
    }

    # Drive a physical-pixel selection through the real overlay message path.
    Send-NativeMessage $overlay 0x0201 1 120 120
    Start-Sleep -Milliseconds 150
    Send-NativeMessage $overlay 0x0200 1 520 520
    Start-Sleep -Milliseconds 150
    Send-NativeMessage $overlay 0x0202 0 520 520
    Start-Sleep -Milliseconds 150
    Start-Sleep -Milliseconds 700

    $scrollWindows = Get-VisibleNativeWindows
    $toolbar = $scrollWindows | Where-Object { $_.Width -gt 250 -and $_.Height -lt 200 } | Select-Object -First 1
    if ($null -eq $toolbar -or $scrollWindows.Count -lt 3) {
        throw "scroll selection did not create the native toolbar, border, and preview windows"
    }
    $status = Send-Frame '{"type":"request","payload":{"requestId":"native-3","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}'
    if ($status -notmatch '"active":true') { throw "native selection did not remain active: $status" }

    # Click the existing Finish button in the native toolbar without opening a save dialog.
    Send-NativeMessage $toolbar 0x0201 1 ((20 * 34) + 17) 23
    Send-NativeMessage $toolbar 0x0202 0 ((20 * 34) + 17) 23
    Start-Sleep -Milliseconds 700
    $finished = Send-Frame '{"type":"request","payload":{"requestId":"native-4","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}'
    $finishedResult = ($finished | ConvertFrom-Json).payload.result
    if ($finishedResult.active) { throw "native finish did not close the session: $finished" }
    if ($null -eq $finishedResult.lastSize -or
        $finishedResult.lastSize.width -ne 400 -or
        $finishedResult.lastSize.height -ne 400) {
        throw "native finish did not publish the selected image size: $finished"
    }

    # Start a second native scroll session and cancel it through the existing toolbar.
    $restart = Send-Frame '{"type":"request","payload":{"requestId":"native-5","pluginId":"screenshot","kind":"call","method":"start_capture","payload":{"mode":"scroll"}}}'
    if ($restart -notmatch '"ok":true') { throw "native restart request failed: $restart" }
    Start-Sleep -Milliseconds 900
    $overlay2 = (Get-VisibleNativeWindows | Sort-Object { $_.Width * $_.Height } -Descending | Select-Object -First 1)
    if ($null -eq $overlay2 -or $overlay2.Width -lt 200 -or $overlay2.Height -lt 200) {
        throw "native restart overlay was not found"
    }
    Send-NativeMessage $overlay2 0x0201 1 120 120
    Send-NativeMessage $overlay2 0x0200 1 520 520
    Send-NativeMessage $overlay2 0x0202 0 520 520
    Start-Sleep -Milliseconds 700
    $toolbar2 = Get-VisibleNativeWindows | Where-Object { $_.Width -gt 250 -and $_.Height -lt 200 } | Select-Object -First 1
    if ($null -eq $toolbar2) { throw "native restart toolbar was not found" }
    Send-NativeMessage $toolbar2 0x0205 0 10 10
    Start-Sleep -Milliseconds 400
    $cancelled = (Send-Frame '{"type":"request","payload":{"requestId":"native-6","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}' | ConvertFrom-Json).payload.result
    if ($cancelled.active) { throw "native toolbar cancel did not close the session" }

    # Start a third session and invoke the existing Copy action. Clipboard contention
    # is reported but the action must still finish and publish the selected size.
    $copyStart = Send-Frame '{"type":"request","payload":{"requestId":"native-7","pluginId":"screenshot","kind":"call","method":"start_capture","payload":{"mode":"scroll"}}}'
    if ($copyStart -notmatch '"ok":true') { throw "native copy-session start failed: $copyStart" }
    Start-Sleep -Milliseconds 900
    $overlay3 = (Get-VisibleNativeWindows | Sort-Object { $_.Width * $_.Height } -Descending | Select-Object -First 1)
    if ($null -eq $overlay3 -or $overlay3.Width -lt 200 -or $overlay3.Height -lt 200) {
        throw "native copy-session overlay was not found"
    }
    Send-NativeMessage $overlay3 0x0201 1 120 120
    Send-NativeMessage $overlay3 0x0200 1 520 520
    Send-NativeMessage $overlay3 0x0202 0 520 520
    Start-Sleep -Milliseconds 700
    $toolbar3 = Get-VisibleNativeWindows | Where-Object { $_.Width -gt 250 -and $_.Height -lt 200 } | Select-Object -First 1
    if ($null -eq $toolbar3) { throw "native copy-session toolbar was not found" }
    Send-NativeMessage $toolbar3 0x0201 1 ((17 * 34) + 17) 23
    Send-NativeMessage $toolbar3 0x0202 0 ((17 * 34) + 17) 23
    Start-Sleep -Milliseconds 500
    $copied = (Send-Frame '{"type":"request","payload":{"requestId":"native-8","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}' | ConvertFrom-Json).payload.result
    if ($copied.active) { throw "native toolbar copy did not close the session" }
    if ($null -eq $copied.lastSize -or
        $copied.lastSize.width -ne 400 -or
        $copied.lastSize.height -ne 400) {
        throw "native toolbar copy did not publish the selected image size"
    }

    # Start a fourth session and exercise the existing Save button plus the
    # standard Win32 save dialog. IDOK handles the save dialog; IDYES handles
    # a possible overwrite confirmation.
    $saveStart = Send-Frame '{"type":"request","payload":{"requestId":"native-10","pluginId":"screenshot","kind":"call","method":"start_capture","payload":{"mode":"scroll"}}}'
    if ($saveStart -notmatch '"ok":true') { throw "native save-session start failed: $saveStart" }
    Start-Sleep -Milliseconds 900
    $overlay4 = (Get-VisibleNativeWindows | Sort-Object { $_.Width * $_.Height } -Descending | Select-Object -First 1)
    if ($null -eq $overlay4 -or $overlay4.Width -lt 200 -or $overlay4.Height -lt 200) {
        throw "native save-session overlay was not found"
    }
    Send-NativeMessage $overlay4 0x0201 1 120 120
    Send-NativeMessage $overlay4 0x0200 1 520 520
    Send-NativeMessage $overlay4 0x0202 0 520 520
    Start-Sleep -Milliseconds 700
    $toolbar4 = Get-VisibleNativeWindows | Where-Object { $_.Width -gt 250 -and $_.Height -lt 200 } | Select-Object -First 1
    if ($null -eq $toolbar4) { throw "native save-session toolbar was not found" }
    # Run the Save click on a background thread: the native handler opens a
    # modal common dialog, so a synchronous SendMessage on this harness thread
    # would block before it can enumerate and dismiss that dialog.
    Send-NativeMessage $toolbar4 0x0201 1 ((18 * 34) + 17) 23
    Send-NativeMessageAsync $toolbar4 0x0202 0 ((18 * 34) + 17) 23
    Start-Sleep -Milliseconds 700
    $saveDialogClosed = $false
    $noDialogStreak = 0
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        $dialogs = @([ScreenshotNativeSmoke]::FindDialogs([uint32]$process.Id))
        foreach ($dialog in $dialogs) {
            # Locate the affirmative Save/Yes button in either the common
            # dialog or its overwrite confirmation and activate it directly.
            $affirmativeButton = [ScreenshotNativeSmoke]::FindAffirmativeButton($dialog)
            if ($affirmativeButton -ne [IntPtr]::Zero) {
                Send-NativeHandleMessageAsync $affirmativeButton 0x00f5 0
            }
        }
        Start-Sleep -Milliseconds 250
        $remainingDialogs = @([ScreenshotNativeSmoke]::FindDialogs([uint32]$process.Id))
        if ($remainingDialogs.Count -eq 0) {
            $noDialogStreak++
            if ($noDialogStreak -ge 4) {
                $saveDialogClosed = $true
                break
            }
        } else {
            $noDialogStreak = 0
        }
    }
    if (-not $saveDialogClosed) { throw "native Save dialog did not close" }
    Start-Sleep -Milliseconds 1000
    $saved = (Send-Frame '{"type":"request","payload":{"requestId":"native-save-status","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}' | ConvertFrom-Json).payload.result
    if ($saved.active) { throw "native Save did not close the capture session" }
    if ($null -eq $saved.lastSize -or
        $saved.lastSize.width -ne 400 -or
        $saved.lastSize.height -ne 400 -or
        [string]::IsNullOrWhiteSpace($saved.lastPath)) {
        throw "native Save did not publish path and selected image size"
    }

    $shutdown = Send-Frame '{"type":"request","payload":{"requestId":"native-9","pluginId":"screenshot","kind":"shutdown","payload":null}}'
    if ($shutdown -notmatch '"ok":true') { throw "plugin shutdown failed: $shutdown" }
    Start-Sleep -Milliseconds 700
    if (Get-Process -Id $process.Id -ErrorAction SilentlyContinue) { throw "plugin did not exit naturally" }
    Write-Output "screenshot-plugin-native-ui-smoke=passed"
}
finally {
    if ($process) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
    $server.Dispose()
    if (Test-Path -LiteralPath $errorPath) {
        $stderr = Get-Content -LiteralPath $errorPath -Raw
        if ($stderr -and $stderr.Trim()) { Write-Output "plugin-stderr=$stderr" }
        Remove-Item -LiteralPath $errorPath -Force -ErrorAction SilentlyContinue
    }
}