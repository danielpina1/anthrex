//! Length-prefixed MessagePack framing: 4-byte big-endian length, then the body.

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest body accepted on either side.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("frame too large: {0} bytes (max {MAX_FRAME})")]
    TooLarge(usize),
    #[error("encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Serializes `msg` and prepends the length header.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, CodecError> {
    let body = rmp_serde::to_vec_named(msg)?;
    if body.len() > MAX_FRAME {
        return Err(CodecError::TooLarge(body.len()));
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Deserializes a body (without the length header).
pub fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, CodecError> {
    Ok(rmp_serde::from_slice(body)?)
}

pub async fn write_frame<W, T>(writer: &mut W, msg: &T) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let frame = encode(msg)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads one frame. Returns `Ok(None)` when the peer closed the stream between frames.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>, CodecError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(CodecError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Some(decode(&body)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientKind, ClientMsg, DaemonMsg};
    use tokio::io::AsyncWriteExt;

    #[test]
    fn encode_prefixes_big_endian_length() {
        let frame = encode(&ClientMsg::ListWindows).unwrap();
        let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(len, frame.len() - 4);
        let back: ClientMsg = decode(&frame[4..]).unwrap();
        assert_eq!(back, ClientMsg::ListWindows);
    }

    #[test]
    fn encode_rejects_oversized_body() {
        let msg = DaemonMsg::Output { window_id: 1, bytes: vec![0u8; MAX_FRAME + 1] };
        assert!(matches!(encode(&msg), Err(CodecError::TooLarge(_))));
    }

    #[tokio::test]
    async fn frames_round_trip_over_a_duplex_stream() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let hello = ClientMsg::Hello { proto_version: 1, client: ClientKind::Cli };
        write_frame(&mut a, &hello).await.unwrap();
        write_frame(&mut a, &ClientMsg::Unsubscribe).await.unwrap();
        let first: Option<ClientMsg> = read_frame(&mut b).await.unwrap();
        let second: Option<ClientMsg> = read_frame(&mut b).await.unwrap();
        assert_eq!(first, Some(hello));
        assert_eq!(second, Some(ClientMsg::Unsubscribe));
    }

    #[tokio::test]
    async fn read_frame_returns_none_on_clean_eof() {
        let (a, mut b) = tokio::io::duplex(64);
        drop(a);
        let got: Option<ClientMsg> = read_frame(&mut b).await.unwrap();
        assert_eq!(got, None);
    }

    #[tokio::test]
    async fn read_frame_rejects_oversized_header() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&((MAX_FRAME as u32) + 1).to_be_bytes()).await.unwrap();
        let got: Result<Option<ClientMsg>, CodecError> = read_frame(&mut b).await;
        assert!(matches!(got, Err(CodecError::TooLarge(_))));
    }
}
