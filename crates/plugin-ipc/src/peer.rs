//! A continuously reading, bidirectional connection. A timeout cancels one wait,
//! never the process, and requests whose delivery is uncertain are never replayed.
use crate::{FrameReader, FrameWriter};
use sideweave_plugin_api::{ErrorCode, PluginError, PluginEvent, PluginId, PluginRequest, PluginResponse, PluginWireMessage};
use std::{collections::HashMap, io::{Read, Write}, sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, mpsc::{sync_channel, SyncSender, RecvTimeoutError}, Arc, Mutex, Weak}, time::Duration};

pub const IPC_MULTIPLEX_CAPABILITY: &str = "ipc.multiplex.v1";
const MAX_PENDING: usize = 128;
const MAX_INCOMING: usize = 64;
type Reply = Result<PluginResponse, PluginError>;
type Writer = FrameWriter<Box<dyn Write + Send>>;
type Handler = dyn Fn(PluginRequest) -> PluginResponse + Send + Sync;
type Events = dyn Fn(PluginEvent) + Send + Sync;

struct Inner {
    id: PluginId,
    writer: Mutex<Writer>,
    pending: Mutex<HashMap<String, SyncSender<Reply>>>,
    closed: AtomicBool,
    incoming: AtomicUsize,
}

#[derive(Clone)]
pub struct RpcPeer(Arc<Inner>);

fn error(code: ErrorCode, message: impl Into<String>) -> PluginError { PluginError { code, message: message.into() } }

impl RpcPeer {
    pub fn start<R: Read + Send + 'static, W: Write + Send + 'static>(
        id: PluginId, reader: R, writer: W, handler: Arc<Handler>, events: Arc<Events>,
    ) -> Self {
        let inner = Arc::new(Inner { id, writer:Mutex::new(FrameWriter::new(Box::new(writer))), pending:Mutex::new(HashMap::new()), closed:AtomicBool::new(false), incoming:AtomicUsize::new(0) });
        let weak = Arc::downgrade(&inner);
        std::thread::spawn(move || read_loop(weak, FrameReader::new(reader), handler, events));
        Self(inner)
    }

    pub fn is_closed(&self) -> bool { self.0.closed.load(Ordering::Acquire) }

    pub fn call(&self, request: PluginRequest) -> Reply {
        request.validate_for(&self.0.id).map_err(|e| error(ErrorCode::InvalidRequest, e.to_string()))?;
        let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(sideweave_plugin_api::DEFAULT_REQUEST_TIMEOUT_MS));
        let id = request.request_id.clone();
        let (sender, receiver) = sync_channel(1);
        {
            let mut pending = self.0.pending.lock().unwrap();
            if self.is_closed() { return Err(error(ErrorCode::PluginUnavailable, "IPC connection is closed")); }
            if pending.len() >= MAX_PENDING { return Err(error(ErrorCode::PluginUnavailable, "Too many pending IPC requests")); }
            if pending.contains_key(&id) { return Err(error(ErrorCode::InvalidRequest, "Duplicate pending request ID")); }
            pending.insert(id.clone(), sender);
        }
        if let Err(e) = self.send(&PluginWireMessage::Request(request)) {
            self.0.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        let result = match receiver.recv_timeout(timeout) {
            Ok(reply) => reply,
            Err(RecvTimeoutError::Timeout) => Err(error(ErrorCode::Timeout, "IPC request timed out; execution may still be in progress")),
            Err(RecvTimeoutError::Disconnected) => Err(error(ErrorCode::PluginUnavailable, "IPC connection was closed")),
        };
        self.0.pending.lock().unwrap().remove(&id);
        result
    }

    pub fn send(&self, message: &PluginWireMessage) -> Result<(), PluginError> {
        if self.is_closed() { return Err(error(ErrorCode::PluginUnavailable, "IPC connection is closed")); }
        let result = self.0.writer.lock().unwrap().write_message(message);
        result.map_err(|e| { self.close(); error(ErrorCode::PluginUnavailable, format!("IPC write failed: {e:?}")) })
    }

    pub fn close(&self) { close(&self.0); }
}

fn close(inner: &Inner) {
    inner.closed.store(true, Ordering::Release);
    let pending = std::mem::take(&mut *inner.pending.lock().unwrap());
    for (_, sender) in pending { let _ = sender.send(Err(error(ErrorCode::PluginUnavailable, "IPC peer disconnected"))); }
}

fn read_loop<R: Read>(weak: Weak<Inner>, mut reader: FrameReader<R>, handler: Arc<Handler>, events: Arc<Events>) {
    loop {
        let message = match reader.read_message() { Ok(message) => message, Err(_) => break };
        let Some(inner) = weak.upgrade() else { break };
        match message {
            PluginWireMessage::Response(response) if response.plugin_id == inner.id => {
                if let Some(sender) = inner.pending.lock().unwrap().remove(&response.request_id) { let _ = sender.send(Ok(response)); }
            }
            PluginWireMessage::Event(event) if event.plugin_id == inner.id => { events(event); }
            PluginWireMessage::Request(request) if request.plugin_id == inner.id => {
                if request.validate_for(&inner.id).is_err() { break; }
                if inner.incoming.fetch_add(1, Ordering::AcqRel) >= MAX_INCOMING {
                    inner.incoming.fetch_sub(1, Ordering::AcqRel);
                    let response = PluginResponse { request_id:request.request_id, plugin_id:request.plugin_id, ok:false, result:serde_json::Value::Null, error:Some(error(ErrorCode::PluginUnavailable, "IPC concurrency limit exceeded")) };
                    let _ = RpcPeer(inner).send(&PluginWireMessage::Response(response));
                    continue;
                }
                let handler = handler.clone();
                std::thread::spawn(move || {
                    let fallback_id = request.request_id.clone();
                    let fallback_plugin = request.plugin_id.clone();
                    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(request))).unwrap_or_else(|_| PluginResponse {
                        request_id:fallback_id, plugin_id:fallback_plugin, ok:false, result:serde_json::Value::Null,
                        error:Some(error(ErrorCode::Internal, "IPC request handler panicked")),
                    });
                    let peer = RpcPeer(inner);
                    let _ = peer.send(&PluginWireMessage::Response(response));
                    peer.0.incoming.fetch_sub(1, Ordering::AcqRel);
                });
            }
            _ => break,
        }
    }
    if let Some(inner) = weak.upgrade() { close(&inner); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sideweave_plugin_api::PluginRequestKind;
    use std::net::{TcpListener, TcpStream};
    fn request(id: &str, timeout: u64) -> PluginRequest {
        PluginRequest { request_id:id.into(), plugin_id:PluginId::new("demo").unwrap(), kind:PluginRequestKind::Call, method:Some("test".into()), payload:serde_json::Value::Null, timeout_ms:Some(timeout) }
    }
    fn response(r: PluginRequest) -> PluginResponse { PluginResponse { request_id:r.request_id, plugin_id:r.plugin_id, ok:true, result:r.payload, error:None } }
    fn pair(handler: Arc<Handler>, events: Arc<Events>) -> (RpcPeer, RpcPeer) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let left = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (right, _) = listener.accept().unwrap();
        let a = RpcPeer::start(PluginId::new("demo").unwrap(), left.try_clone().unwrap(), left, Arc::new(response), events);
        let b = RpcPeer::start(PluginId::new("demo").unwrap(), right.try_clone().unwrap(), right, handler, Arc::new(|_| {}));
        (a,b)
    }
    #[test]
    fn receives_events_while_idle() {
        let (tx,rx) = sync_channel(1);
        let (a,b) = pair(Arc::new(response), Arc::new(move |e| { tx.send(e.topic).unwrap(); }));
        b.send(&PluginWireMessage::Event(PluginEvent { plugin_id:PluginId::new("demo").unwrap(), topic:"background".into(), payload:serde_json::Value::Null })).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "background"); a.close(); b.close();
    }
    #[test]
    fn timeout_does_not_kill_other_requests_or_replay() {
        let count = Arc::new(AtomicUsize::new(0)); let observed = count.clone();
        let (a,b) = pair(Arc::new(move |r| { observed.fetch_add(1,Ordering::SeqCst); if r.request_id == "slow" { std::thread::sleep(Duration::from_millis(150)); } response(r) }),Arc::new(|_| {}));
        let slow = a.clone(); let task = std::thread::spawn(move || slow.call(request("slow",30)));
        assert!(a.call(request("fast",1000)).unwrap().ok);
        assert!(matches!(task.join().unwrap().unwrap_err().code, ErrorCode::Timeout));
        std::thread::sleep(Duration::from_millis(200));
        assert!(a.call(request("after",1000)).unwrap().ok);
        assert_eq!(count.load(Ordering::SeqCst),3); a.close();b.close();
    }
    #[test]
    fn close_fails_pending_waiters_promptly() {
        let (a,b) = pair(Arc::new(|r| { std::thread::sleep(Duration::from_millis(200)); response(r) }),Arc::new(|_| {}));
        let caller=a.clone(); let task=std::thread::spawn(move || caller.call(request("pending",1000)));
        std::thread::sleep(Duration::from_millis(20)); a.close();
        assert!(matches!(task.join().unwrap().unwrap_err().code, ErrorCode::PluginUnavailable));b.close();
    }

    #[test]
    fn dispatches_out_of_order_replies_and_nested_callbacks(){
        let callback:Arc<std::sync::OnceLock<RpcPeer>>=Arc::new(std::sync::OnceLock::new());let server=callback.clone();
        let(a,b)=pair(Arc::new(move|r|{
            if r.request_id=="outer"{let reply=server.wait().call(request("callback",1000)).unwrap();assert!(reply.ok);std::thread::sleep(Duration::from_millis(80));}
            response(r)
        }),Arc::new(|_|{}));callback.set(b.clone()).ok().unwrap();
        let caller=a.clone();let outer=std::thread::spawn(move||caller.call(request("outer",2000)));
        let fast=a.call(request("parallel",1000)).unwrap();assert_eq!(fast.request_id,"parallel");
        assert_eq!(outer.join().unwrap().unwrap().request_id,"outer");a.close();b.close();
    }
}
