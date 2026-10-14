param(
    [string]$PluginExecutable = "$env:APPDATA\com.agiletiles.app\user_plugins\screenshot\agile-tiles-screenshot-core.exe"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $PluginExecutable -PathType Leaf)) {
    throw "screenshot plugin executable not found: $PluginExecutable"
}

$pipeName = "agiletiles-screenshot-smoke-$([guid]::NewGuid().ToString('N'))"
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

try {
    $process = Start-Process -FilePath $PluginExecutable `
        -ArgumentList @("--pipe", $pipePath, "--plugin-id", "screenshot") `
        -PassThru -WindowStyle Hidden -RedirectStandardError $errorPath
    $wait = $server.BeginWaitForConnection($null, $null)
    if (-not $wait.AsyncWaitHandle.WaitOne(5000)) {
        throw "IPC connection timed out"
    }
    $server.EndWaitForConnection($wait)

    $handshake = Send-Frame '{"type":"handshake","payload":{"requestId":"smoke-1","pluginId":"screenshot","apiVersion":1,"hostVersion":"smoke","capabilities":[]}}'
    $start = Send-Frame '{"type":"request","payload":{"requestId":"smoke-2","pluginId":"screenshot","kind":"call","method":"start_capture","payload":{"mode":"scroll"}}}'
    Start-Sleep -Milliseconds 400
    $status = Send-Frame '{"type":"request","payload":{"requestId":"smoke-3","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}'
    $cancel = Send-Frame '{"type":"request","payload":{"requestId":"smoke-4","pluginId":"screenshot","kind":"call","method":"cancel_capture","payload":null}}'
    Start-Sleep -Milliseconds 400
    $cancelStatus = Send-Frame '{"type":"request","payload":{"requestId":"smoke-5","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}'
    $restart = Send-Frame '{"type":"request","payload":{"requestId":"smoke-6","pluginId":"screenshot","kind":"call","method":"start_capture","payload":{"mode":"scroll"}}}'
    Start-Sleep -Milliseconds 400
    $restartStatus = Send-Frame '{"type":"request","payload":{"requestId":"smoke-7","pluginId":"screenshot","kind":"call","method":"get_status","payload":null}}'
    $shutdown = Send-Frame '{"type":"request","payload":{"requestId":"smoke-8","pluginId":"screenshot","kind":"shutdown","payload":null}}'

    if ($handshake -notmatch '"accepted":true') { throw "plugin handshake was not accepted" }
    if ($start -notmatch '"ok":true') { throw "scroll start request failed: $start" }
    if ($status -notmatch '"active":true') { throw "plugin did not report active status: $status" }
    if ($cancel -notmatch '"ok":true') { throw "plugin cancel request failed: $cancel" }
    if ($cancelStatus -notmatch '"active":false') { throw "plugin did not become inactive after cancel: $cancelStatus" }
    if ($restart -notmatch '"ok":true') { throw "plugin restart request failed: $restart" }
    if ($restartStatus -notmatch '"active":true') { throw "plugin did not report active status after restart: $restartStatus" }
    if ($shutdown -notmatch '"ok":true') { throw "plugin shutdown request failed: $shutdown" }

    Start-Sleep -Milliseconds 700
    if (Get-Process -Id $process.Id -ErrorAction SilentlyContinue) {
        throw "plugin did not exit naturally after shutdown"
    }
    Write-Output "screenshot-plugin-ipc-smoke=passed"
}
finally {
    if ($process) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
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
