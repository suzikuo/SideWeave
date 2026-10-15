#![cfg(windows)]

use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginEvent, PluginHandshake, PluginId, PluginManifest, PluginRequest,
    PluginRequestKind, PluginResponse, PluginWireMessage, API_VERSION, DEFAULT_REQUEST_TIMEOUT_MS,
};
use sideweave_plugin_ipc::{FrameReader, FrameWriter};
use sideweave_plugin_runtime::ExternalPlugin;
use sideweave_plugin_sdk::{DesktopNotificationOptions, HostServices, NotificationOptions};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::ptr::null_mut;
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, SetNamedPipeHandleState, PIPE_NOWAIT, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PIPE_BUFFER_SIZE: u32 = 512 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ExternalPluginProcess {
    plugin_id: PluginId,
    manifest: PluginManifest,
    executable: PathBuf,
    args: Vec<String>,
    working_dir: Option<PathBuf>,
    transport: Option<ExternalTransport>,
    child: Option<Child>,
    job: Option<JobObject>,
    services: Arc<dyn HostServices>,
}

impl ExternalPluginProcess {
    pub fn start_with_working_dir(
        manifest: &PluginManifest,
        executable: impl AsRef<Path>,
        args: &[String],
        services: Arc<dyn HostServices>,
        working_dir: Option<&Path>,
    ) -> Result<Self, PluginError> {
        let executable = executable.as_ref().to_path_buf();
        let args = args.to_vec();
        let working_dir = working_dir.map(Path::to_path_buf);
        let spawned = spawn_process(
            manifest,
            &executable,
            &args,
            working_dir.as_deref(),
            Arc::clone(&services),
        )?;
        Ok(Self {
            plugin_id: manifest.id.clone(),
            manifest: manifest.clone(),
            executable,
            args,
            working_dir,
            transport: Some(spawned.transport),
            child: Some(spawned.child),
            job: Some(spawned.job),
            services,
        })
    }

    fn ensure_running(&mut self) -> Result<(), PluginError> {
        if self.transport.is_some() {
            return Ok(());
        }
        let spawned = spawn_process(
            &self.manifest,
            &self.executable,
            &self.args,
            self.working_dir.as_deref(),
            Arc::clone(&self.services),
        )?;
        self.transport = Some(spawned.transport);
        self.child = Some(spawned.child);
        self.job = Some(spawned.job);
        Ok(())
    }

    fn stop_process(&mut self) {
        self.transport.take();
        if let Some(mut child) = self.child.take() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!(
                        "[sideweave_] external plugin {} already exited: {}",
                        self.plugin_id, status
                    );
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                Err(error) => {
                    eprintln!(
                        "[sideweave_] failed to query external plugin {}: {}",
                        self.plugin_id, error
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        self.job.take();
    }
}
impl Drop for ExternalPluginProcess {
    fn drop(&mut self) {
        self.stop_process();
    }
}

struct SpawnedProcess {
    transport: ExternalTransport,
    child: Child,
    job: JobObject,
}

struct ExternalTransport {
    plugin_id: PluginId,
    permissions: Vec<String>,
    reader: FrameReader<File>,
    writer: FrameWriter<File>,
    services: Arc<dyn HostServices>,
}

fn spawn_process(
    manifest: &PluginManifest,
    executable: &Path,
    args: &[String],
    working_dir: Option<&Path>,
    services: Arc<dyn HostServices>,
) -> Result<SpawnedProcess, PluginError> {
    let pipe_name = format!(r"\\.\pipe\agiletiles-{}-{}", manifest.id, unique_id());
    let server = create_pipe(&pipe_name).map_err(internal_error)?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .arg("--pipe")
        .arg(&pipe_name)
        .arg("--plugin-id")
        .arg(manifest.id.as_str());
    if let Some(working_dir) = working_dir {
        command.current_dir(working_dir);
    }
    use std::os::windows::process::CommandExt;
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|error| internal_error(error.to_string()))?;
    let job = match JobObject::for_child(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            return Err(internal_error(error));
        }
    };

    let file = match connect_pipe(server) {
        Ok(file) => file,
        Err(error) => {
            let _ = child.kill();
            return Err(internal_error(error));
        }
    };
    let reader_file = match file.try_clone() {
        Ok(file) => file,
        Err(error) => {
            let _ = child.kill();
            return Err(internal_error(error.to_string()));
        }
    };
    let reader = FrameReader::new(reader_file);
    let mut writer = FrameWriter::new(file);
    let handshake = PluginWireMessage::Handshake(PluginHandshake {
        request_id: unique_id(),
        plugin_id: manifest.id.clone(),
        api_version: API_VERSION,
        host_version: env!("CARGO_PKG_VERSION").into(),
        capabilities: manifest.permissions.clone(),
    });
    if let Err(error) = writer.write_message(&handshake) {
        let _ = child.kill();
        let _ = child.wait();
        drop(job);
        return Err(internal_error(format!(
            "IPC handshake write failed: {error:?}"
        )));
    }
    let (reader, response) = match read_handshake_with_timeout(reader, STARTUP_TIMEOUT) {
        Ok(result) => result,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            drop(writer);
            drop(job);
            return Err(error);
        }
    };
    match response {
        PluginWireMessage::HandshakeResponse(response)
            if response.accepted
                && response.request_id == handshake_request_id(&handshake)
                && response.plugin_id == manifest.id =>
        {
            Ok(SpawnedProcess {
                transport: ExternalTransport {
                    plugin_id: manifest.id.clone(),
                    permissions: manifest.permissions.clone(),
                    reader,
                    writer,
                    services,
                },
                child,
                job,
            })
        }
        PluginWireMessage::HandshakeResponse(response) => Err(response
            .error
            .unwrap_or_else(|| internal_error("external plugin rejected handshake".into()))),
        _ => Err(internal_error(
            "invalid external plugin handshake response".into(),
        )),
    }
}

fn handshake_request_id(message: &PluginWireMessage) -> String {
    match message {
        PluginWireMessage::Handshake(handshake) => handshake.request_id.clone(),
        _ => String::new(),
    }
}

fn read_handshake_with_timeout(
    mut reader: FrameReader<File>,
    timeout: Duration,
) -> Result<(FrameReader<File>, PluginWireMessage), PluginError> {
    let (sender, receiver) = sync_channel(1);
    std::thread::spawn(move || {
        let result = reader.read_message();
        let _ = sender.send((reader, result));
    });
    match receiver.recv_timeout(timeout) {
        Ok((reader, Ok(message))) => Ok((reader, message)),
        Ok((_, Err(error))) => Err(internal_error(format!(
            "IPC handshake read failed: {error:?}"
        ))),
        Err(RecvTimeoutError::Timeout) => Err(PluginError {
            code: ErrorCode::Timeout,
            message: "external plugin handshake timed out".into(),
        }),
        Err(RecvTimeoutError::Disconnected) => Err(internal_error(
            "external plugin handshake reader exited".into(),
        )),
    }
}

impl ExternalTransport {
    fn handle_host_call(&self, request: PluginRequest) -> PluginResponse {
        if let Err(error) = request.validate_for(&self.plugin_id) {
            return PluginResponse {
                request_id: request.request_id,
                plugin_id: request.plugin_id,
                ok: false,
                result: serde_json::Value::Null,
                error: Some(PluginError {
                    code: ErrorCode::InvalidRequest,
                    message: error.to_string(),
                }),
            };
        }
        let result = self.dispatch_host_call(&request);
        match result {
            Ok(value) => success_response(&request, value),
            Err(error) => PluginResponse {
                request_id: request.request_id,
                plugin_id: request.plugin_id,
                ok: false,
                result: serde_json::Value::Null,
                error: Some(error),
            },
        }
    }

    fn dispatch_host_call(
        &self,
        request: &PluginRequest,
    ) -> Result<serde_json::Value, PluginError> {
        let method = request.method.as_deref().unwrap_or_default();
        if let Some(permission) = sideweave_plugin_api::permissions::for_host_method(method) {
            if !sideweave_plugin_api::permissions::is_declared(&self.permissions, permission) {
                return Err(permission_denied(format!(
                    "plugin did not declare permission: {permission}"
                )));
            }
            if !self
                .services
                .permission_granted(&self.plugin_id, permission)
                .map_err(host_error)?
            {
                return Err(permission_denied(format!(
                    "permission denied by user: {permission}"
                )));
            }
        }
        let payload = &request.payload;
        match method {
            "host.storage.get" => {
                let key = string_field(payload, "key")?;
                self.services
                    .storage()
                    .get(&self.plugin_id, &key)
                    .map(|value| serde_json::json!(value))
                    .map_err(host_error)
            }
            "host.storage.set" => {
                let key = string_field(payload, "key")?;
                let value = string_field(payload, "value")?;
                self.services
                    .storage()
                    .set(&self.plugin_id, &key, &value)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.storage.remove" => {
                let key = string_field(payload, "key")?;
                self.services
                    .storage()
                    .remove(&self.plugin_id, &key)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.secrets.get" => {
                let key = string_field(payload, "key")?;
                self.services
                    .secrets()
                    .get(&self.plugin_id, &key)
                    .map(|value| serde_json::json!(value))
                    .map_err(host_error)
            }
            "host.secrets.set" => {
                let key = string_field(payload, "key")?;
                let value = string_field(payload, "value")?;
                self.services
                    .secrets()
                    .set(&self.plugin_id, &key, &value)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.secrets.remove" => {
                let key = string_field(payload, "key")?;
                self.services
                    .secrets()
                    .remove(&self.plugin_id, &key)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.system.open_external" => {
                let url = string_field(payload, "url")?;
                self.services
                    .system()
                    .open_external(&url)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.notification.notify" => {
                let notification: NotificationOptions = serde_json::from_value(payload.clone())
                    .map_err(|error| invalid_request(error.to_string()))?;
                self.services
                    .notification()
                    .notify_with(&notification)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.notification.show_desktop" => {
                let notification: DesktopNotificationOptions =
                    serde_json::from_value(payload.clone())
                        .map_err(|error| invalid_request(error.to_string()))?;
                self.services
                    .notification()
                    .show_desktop_for_plugin(&self.plugin_id, &notification)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.notification.show_sidebar" => {
                let notification: DesktopNotificationOptions =
                    serde_json::from_value(payload.clone())
                        .map_err(|error| invalid_request(error.to_string()))?;
                self.services
                    .notification()
                    .show_sidebar_for_plugin(&self.plugin_id, &notification)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.system.spawn" => {
                let program = string_field(payload, "program")?;
                let args = string_array_field(payload, "args")?;
                self.services
                    .system()
                    .spawn(&program, &args)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.system.spawn_with_cwd" => {
                let program = string_field(payload, "program")?;
                let args = string_array_field(payload, "args")?;
                let cwd = payload.get("cwd").and_then(|value| value.as_str());
                self.services
                    .system()
                    .spawn_with_cwd(&program, &args, cwd)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.system.pick_file" => {
                let extensions = string_array_field(payload, "extensions")?;
                self.services
                    .system()
                    .pick_file(&extensions)
                    .map(|value| serde_json::json!(value))
                    .map_err(host_error)
            }
            "host.system.extract_icon" => {
                let path = string_field(payload, "path")?;
                self.services
                    .system()
                    .extract_icon(&path)
                    .map(|value| serde_json::json!(value))
                    .map_err(host_error)
            }
            "host.files.read_private" => {
                let path = string_field(payload, "path")?;
                self.services
                    .files()
                    .read_private(&self.plugin_id, &path)
                    .map(|value| serde_json::json!(value.map(|bytes| BASE64.encode(bytes))))
                    .map_err(host_error)
            }
            "host.files.write_private" => {
                let path = string_field(payload, "path")?;
                let encoded = string_field(payload, "bytes")?;
                let bytes = BASE64
                    .decode(encoded)
                    .map_err(|error| invalid_request(format!("invalid file bytes: {error}")))?;
                self.services
                    .files()
                    .write_private(&self.plugin_id, &path, &bytes)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.files.remove_private" => {
                let path = string_field(payload, "path")?;
                self.services
                    .files()
                    .remove_private(&self.plugin_id, &path)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.files.read_external" => {
                let path = string_field(payload, "path")?;
                self.services
                    .files()
                    .read_external(&path)
                    .map(|bytes| serde_json::json!(BASE64.encode(bytes)))
                    .map_err(host_error)
            }
            "host.files.private_path" => {
                let path = string_field(payload, "path")?;
                self.services
                    .files()
                    .private_path(&self.plugin_id, &path)
                    .map(serde_json::Value::String)
                    .map_err(host_error)
            }
            "host.clipboard.read" => self
                .services
                .clipboard()
                .read_text()
                .map(|value| serde_json::json!(value))
                .map_err(host_error),
            "host.clipboard.write" => {
                let text = string_field(payload, "text")?;
                self.services
                    .clipboard()
                    .write_text(&text)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            "host.events.publish" => {
                let event: PluginEvent = serde_json::from_value(
                    payload
                        .get("event")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                )
                .map_err(|error| invalid_request(error.to_string()))?;
                self.services
                    .events()
                    .publish(event)
                    .map(|_| serde_json::Value::Null)
                    .map_err(host_error)
            }
            _ => Err(PluginError {
                code: ErrorCode::PermissionDenied,
                message: format!("unsupported host service: {method}"),
            }),
        }
    }

    fn call(&mut self, request: PluginRequest) -> Result<PluginResponse, PluginError> {
        self.writer
            .write_message(&PluginWireMessage::Request(request.clone()))
            .map_err(|error| internal_error(format!("IPC request write failed: {error:?}")))?;
        loop {
            let message = self
                .reader
                .read_message()
                .map_err(|error| internal_error(format!("IPC response read failed: {error:?}")))?;
            match message {
                PluginWireMessage::Response(response)
                    if response.request_id == request.request_id =>
                {
                    return Ok(response);
                }
                PluginWireMessage::Request(host_request)
                    if host_request
                        .method
                        .as_deref()
                        .unwrap_or_default()
                        .starts_with("host.") =>
                {
                    self.writer
                        .write_message(&PluginWireMessage::Response(
                            self.handle_host_call(host_request),
                        ))
                        .map_err(|error| {
                            internal_error(format!("IPC host response write failed: {error:?}"))
                        })?;
                }
                PluginWireMessage::Event(event) => {
                    let event_permission = sideweave_plugin_api::permissions::EVENT_PUBLISH;
                    if sideweave_plugin_api::permissions::is_declared(
                        &self.permissions,
                        event_permission,
                    ) && matches!(
                        self.services
                            .permission_granted(&self.plugin_id, event_permission),
                        Ok(true)
                    ) {
                        let _ = self.services.events().publish(event);
                    }
                }
                _ => {
                    return Err(internal_error(
                        "unexpected external plugin IPC message".into(),
                    ))
                }
            }
        }
    }
}

impl ExternalPlugin for ExternalPluginProcess {
    fn call(&mut self, request: PluginRequest) -> Result<PluginResponse, PluginError> {
        self.ensure_running()?;
        match self.call_once(request.clone()) {
            Ok(response) => Ok(response),
            Err(error) if is_recoverable_ipc_error(&error) => {
                eprintln!(
                    "[sideweave_] external plugin {} IPC disconnected; restarting once: {}",
                    self.plugin_id, error.message
                );
                self.stop_process();
                self.ensure_running().map_err(|restart| {
                    internal_error(format!(
                        "{}; restart failed: {}",
                        error.message, restart.message
                    ))
                })?;
                self.call_once(request)
            }
            Err(error) => Err(error),
        }
    }

    fn shutdown(&mut self) -> Result<(), PluginError> {
        let request = PluginRequest {
            request_id: unique_id(),
            plugin_id: self.plugin_id.clone(),
            kind: PluginRequestKind::Shutdown,
            method: None,
            payload: serde_json::Value::Null,
            timeout_ms: None,
        };
        let _ = self.call(request);
        self.stop_process();
        Ok(())
    }
}

impl ExternalPluginProcess {
    fn call_once(&mut self, request: PluginRequest) -> Result<PluginResponse, PluginError> {
        let transport = self
            .transport
            .take()
            .ok_or_else(|| internal_error("external plugin transport is unavailable".into()))?;
        let timeout =
            Duration::from_millis(request.timeout_ms.unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS));
        let (sender, receiver) = sync_channel(1);
        std::thread::spawn(move || {
            let mut transport = transport;
            let result = transport.call(request);
            let _ = sender.send((transport, result));
        });
        match receiver.recv_timeout(timeout) {
            Ok((transport, result)) => {
                if result.is_err() {
                    self.stop_process();
                } else {
                    self.transport = Some(transport);
                }
                result
            }
            Err(RecvTimeoutError::Timeout) => {
                self.stop_process();
                Err(PluginError {
                    code: ErrorCode::Timeout,
                    message: "external plugin request timed out".into(),
                })
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.stop_process();
                Err(internal_error("external plugin process exited".into()))
            }
        }
    }
}
fn string_field(payload: &serde_json::Value, key: &str) -> Result<String, PluginError> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| invalid_request(format!("missing field: {key}")))
}

fn string_array_field(payload: &serde_json::Value, key: &str) -> Result<Vec<String>, PluginError> {
    payload
        .get(key)
        .and_then(|value| value.as_array())
        .ok_or_else(|| invalid_request(format!("missing field: {key}")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid_request(format!("field {key} must contain strings")))
        })
        .collect()
}

fn invalid_request(message: String) -> PluginError {
    PluginError {
        code: ErrorCode::InvalidRequest,
        message,
    }
}

fn host_error(error: sideweave_plugin_sdk::HostError) -> PluginError {
    PluginError {
        code: ErrorCode::Internal,
        message: error.message,
    }
}

fn permission_denied(message: String) -> PluginError {
    PluginError {
        code: ErrorCode::PermissionDenied,
        message,
    }
}

fn is_recoverable_ipc_error(error: &PluginError) -> bool {
    let message = error.message.to_ascii_lowercase();
    message.contains("brokenpipe")
        || message.contains("broken pipe")
        || message.contains("ipc request write failed")
        || message.contains("ipc response read failed")
        || message.contains("ipc host response write failed")
        || message.contains("external plugin process exited")
}
fn internal_error(message: String) -> PluginError {
    PluginError {
        code: ErrorCode::PluginUnavailable,
        message,
    }
}

fn unique_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("ipc-{nanos:x}")
}

fn create_pipe(name: &str) -> Result<File, String> {
    let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            5_000,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(unsafe { File::from_raw_handle(handle as _) })
}

fn connect_pipe(file: File) -> Result<File, String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let connected =
            unsafe { ConnectNamedPipe(file.as_raw_handle() as HANDLE, null_mut()) } != 0;
        if connected {
            break;
        }
        let error = unsafe { GetLastError() };
        if error == ERROR_PIPE_CONNECTED {
            break;
        }
        if error != ERROR_PIPE_LISTENING && error != ERROR_NO_DATA {
            return Err(io::Error::last_os_error().to_string());
        }
        if Instant::now() >= deadline {
            return Err("external plugin Named Pipe connection timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let wait_mode = PIPE_WAIT;
    let switched = unsafe {
        SetNamedPipeHandleState(
            file.as_raw_handle() as HANDLE,
            &wait_mode,
            null_mut(),
            null_mut(),
        )
    } != 0;
    if !switched {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(file)
}

struct JobObject(HANDLE);

unsafe impl Send for JobObject {}

impl JobObject {
    fn for_child(child: &Child) -> Result<Self, String> {
        let job = unsafe { CreateJobObjectW(null_mut(), null_mut()) };
        if job.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        } != 0;
        let assigned =
            unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) } != 0;
        if !configured || !assigned {
            unsafe { CloseHandle(job) };
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(Self(job))
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn success_response(request: &PluginRequest, result: serde_json::Value) -> PluginResponse {
    PluginResponse {
        request_id: request.request_id.clone(),
        plugin_id: request.plugin_id.clone(),
        ok: true,
        result,
        error: None,
    }
}
