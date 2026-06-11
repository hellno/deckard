//! Length-delimited CBOR framing for the UDS wire: a **4-byte big-endian length prefix**
//! followed by the CBOR body, one request/response per frame. Frames over [`MAX_FRAME`]
//! (1 MiB) are rejected — a hostile or buggy client can't make the daemon allocate
//! unbounded memory.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Hard cap on a single frame body. The 4-byte prefix can express up to 4 GiB; we refuse
/// anything past 1 MiB (the largest legitimate frame — a big `calldata` — is far smaller).
pub const MAX_FRAME: usize = 1024 * 1024;

/// Read one frame. Returns `Ok(None)` on a clean EOF (peer closed between frames) so the
/// connection loop can exit quietly; any other short read is an error.
pub async fn read_frame<R>(r: &mut R) -> anyhow::Result<Option<Vec<u8>>>
where
    R: AsyncReadExt + Unpin,
{
    // Read the length prefix one fill at a time so we can tell a clean between-frames EOF
    // (zero bytes before any prefix byte) from a malformed one (peer sent 1-3 prefix bytes then
    // hung up). `read_exact` collapses both into `UnexpectedEof`, which would mis-report a
    // truncated prefix as a clean close.
    let mut len_buf = [0u8; 4];
    let mut filled = 0;
    while filled < len_buf.len() {
        let n = r.read(&mut len_buf[filled..]).await?;
        if n == 0 {
            if filled == 0 {
                return Ok(None); // clean EOF: peer closed between frames
            }
            anyhow::bail!("truncated length prefix: {filled} of 4 bytes before EOF");
        }
        filled += n;
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(
        len <= MAX_FRAME,
        "frame too large: {len} bytes > {MAX_FRAME}"
    );
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(Some(body))
}

/// Write one frame (length prefix + body), then flush.
pub async fn write_frame<W>(w: &mut W, body: &[u8]) -> anyhow::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    anyhow::ensure!(
        body.len() <= MAX_FRAME,
        "frame too large: {} bytes",
        body.len()
    );
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

/// CBOR-encode a value into a frame body.
pub fn encode<T: serde::Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf).map_err(|e| anyhow::anyhow!("cbor encode: {e}"))?;
    anyhow::ensure!(
        buf.len() <= MAX_FRAME,
        "encoded frame too large: {} bytes",
        buf.len()
    );
    Ok(buf)
}

/// CBOR-decode a frame body into a value.
pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<T> {
    ciborium::from_reader(bytes).map_err(|e| anyhow::anyhow!("cbor decode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_a_frame() {
        let payload = b"hello deckard".to_vec();
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &payload).await.unwrap();
        // 4-byte prefix + body.
        assert_eq!(buf.len(), 4 + payload.len());
        assert_eq!(&buf[0..4], &(payload.len() as u32).to_be_bytes());

        let mut cursor = std::io::Cursor::new(buf);
        let got = read_frame(&mut cursor).await.unwrap().unwrap();
        assert_eq!(got, payload);
    }

    #[tokio::test]
    async fn clean_eof_is_none() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        assert!(read_frame(&mut cursor).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn partial_length_prefix_is_an_error() {
        // 1-3 bytes of the 4-byte prefix then EOF is a truncated frame, NOT a clean close
        // (only a zero-byte read BEFORE any prefix byte is clean).
        for partial in [vec![0u8], vec![0u8, 0u8], vec![0u8, 0u8, 1u8]] {
            let mut cursor = std::io::Cursor::new(partial);
            assert!(
                read_frame(&mut cursor).await.is_err(),
                "a truncated length prefix must be an error, not a clean EOF"
            );
        }
    }

    #[tokio::test]
    async fn oversize_length_is_rejected() {
        // A 4-byte prefix claiming > 1 MiB must be refused before allocating the body.
        let mut framed = ((MAX_FRAME as u32) + 1).to_be_bytes().to_vec();
        framed.extend_from_slice(&[0u8; 8]); // some body bytes (never fully read)
        let mut cursor = std::io::Cursor::new(framed);
        assert!(read_frame(&mut cursor).await.is_err());
    }

    #[tokio::test]
    async fn cbor_encode_decode_round_trips() {
        let value = ("send", 42u64, true);
        let body = encode(&value).unwrap();
        let back: (String, u64, bool) = decode(&body).unwrap();
        assert_eq!(back, ("send".to_string(), 42, true));
    }
}
