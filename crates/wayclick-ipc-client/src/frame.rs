// SPDX-License-Identifier: MIT
//! Length-prefixed JSON-RPC 2.0 frame protocol.
//!
//! Wire format: 4-byte big-endian length prefix + UTF-8 JSON payload.
//! Maximum frame size is [`MAX_FRAME_SIZE`] bytes; larger frames are rejected
//! to bound memory use against malicious or buggy peers.

use serde_json::Value;
use std::io::{self, Read, Write};
use thiserror::Error;

/// Maximum size of a single IPC frame in bytes.
pub const MAX_FRAME_SIZE: u32 = 65536;

/// Errors produced by frame encoding / decoding and clients built on top.
#[derive(Debug, Error)]
pub enum IpcError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Frame too large: {0} bytes (max 65536)")]
    FrameTooLarge(u32),
    #[error("Connection closed")]
    ConnectionClosed,
}

/// Encode a JSON value into a length-prefixed frame.
///
/// Returns [`IpcError::FrameTooLarge`] if the encoded JSON exceeds
/// [`MAX_FRAME_SIZE`] bytes.
pub fn encode_frame(payload: &Value) -> Result<Vec<u8>, IpcError> {
    let json_bytes = serde_json::to_vec(payload)?;
    let len = json_bytes.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(IpcError::FrameTooLarge(len));
    }
    let mut frame = Vec::with_capacity(4 + json_bytes.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&json_bytes);
    Ok(frame)
}

/// Decode one length-prefixed frame from a reader.
///
/// Reads exactly `4 + length` bytes. Returns [`IpcError::ConnectionClosed`]
/// when the reader returns EOF before the length prefix is complete.
pub fn decode_frame(reader: &mut impl Read) -> Result<Value, IpcError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            IpcError::ConnectionClosed
        } else {
            IpcError::Io(e)
        }
    })?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        return Err(IpcError::FrameTooLarge(len));
    }
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    let value: Value = serde_json::from_slice(&payload)?;
    Ok(value)
}

/// Encode `payload` and write the resulting frame to `writer`, flushing on success.
pub fn write_frame(writer: &mut impl Write, payload: &Value) -> Result<(), IpcError> {
    let frame = encode_frame(payload)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

/// Convenience alias for [`decode_frame`]. Provided for symmetry with [`write_frame`].
pub fn read_frame(reader: &mut impl Read) -> Result<Value, IpcError> {
    decode_frame(reader)
}
