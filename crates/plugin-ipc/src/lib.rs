//! Length-delimited transport frames shared by the host and external plugins.
//!
//! The transport is deliberately independent from Windows Named Pipe handles. The host and
//! plugin process can use the same codec over a pipe, while tests can use an in-memory stream.

use sideweave_plugin_api::{PluginWireMessage, ProtocolError, MAX_MESSAGE_BYTES};
use std::io::{self, Read, Write};

pub const FRAME_HEADER_BYTES: usize = 4;

pub fn encode_frame(message: &PluginWireMessage) -> Result<Vec<u8>, ProtocolError> {
    let payload = message.encode()?;
    let length =
        u32::try_from(payload.len()).map_err(|_| ProtocolError::MessageTooLarge(payload.len()))?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_frame(frame: &[u8]) -> Result<PluginWireMessage, ProtocolError> {
    if frame.len() < FRAME_HEADER_BYTES {
        return Err(ProtocolError::Serialization("incomplete IPC frame".into()));
    }
    let length = u32::from_le_bytes(frame[..FRAME_HEADER_BYTES].try_into().unwrap()) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::MessageTooLarge(length));
    }
    if frame.len() != FRAME_HEADER_BYTES + length {
        return Err(ProtocolError::Serialization(
            "IPC frame length mismatch".into(),
        ));
    }
    PluginWireMessage::decode(&frame[FRAME_HEADER_BYTES..])
}

pub struct FrameReader<R> {
    reader: R,
}

impl<R: Read> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn read_message(&mut self) -> Result<PluginWireMessage, IpcError> {
        let mut header = [0u8; FRAME_HEADER_BYTES];
        self.reader.read_exact(&mut header)?;
        let length = u32::from_le_bytes(header) as usize;
        if length > MAX_MESSAGE_BYTES {
            return Err(IpcError::Protocol(ProtocolError::MessageTooLarge(length)));
        }
        let mut payload = vec![0u8; length];
        self.reader.read_exact(&mut payload)?;
        PluginWireMessage::decode(&payload).map_err(IpcError::Protocol)
    }
}

pub struct FrameWriter<W> {
    writer: W,
}

impl<W: Write> FrameWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub fn write_message(&mut self, message: &PluginWireMessage) -> Result<(), IpcError> {
        let frame = encode_frame(message).map_err(IpcError::Protocol)?;
        self.writer.write_all(&frame)?;
        // NOTE: We do not call self.writer.flush() here. On Windows, calling flush()
        // on a Named Pipe File invokes FlushFileBuffers, which blocks synchronously
        // until the receiving end drains the pipe. If the peer is not actively reading,
        // FlushFileBuffers causes an indefinite thread hang.
        Ok(())
    }
}

#[derive(Debug)]
pub enum IpcError {
    Io(io::Error),
    Protocol(ProtocolError),
}

impl From<io::Error> for IpcError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sideweave_plugin_api::{PluginHandshake, PluginId, API_VERSION};
    use std::io::Cursor;

    #[test]
    fn round_trips_length_delimited_message() {
        let message = PluginWireMessage::Handshake(PluginHandshake {
            request_id: "hello".into(),
            plugin_id: PluginId::new("demo").unwrap(),
            api_version: API_VERSION,
            host_version: "test".into(),
            capabilities: vec!["plugin.call".into()],
        });
        let frame = encode_frame(&message).unwrap();
        assert!(matches!(
            decode_frame(&frame).unwrap(),
            PluginWireMessage::Handshake(_)
        ));

        let mut output = Vec::new();
        FrameWriter::new(&mut output)
            .write_message(&message)
            .unwrap();
        let decoded = FrameReader::new(Cursor::new(output))
            .read_message()
            .unwrap();
        assert!(matches!(decoded, PluginWireMessage::Handshake(_)));
    }

    #[test]
    fn rejects_mismatched_frame_length() {
        let mut frame = vec![5, 0, 0, 0, b'x'];
        assert!(decode_frame(&frame).is_err());
        frame[0] = 1;
        assert!(decode_frame(&frame).is_err());
    }
}
