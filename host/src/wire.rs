//! The tooob protocol's framing, shared by the Host and Visitors (the browser shell speaks it too).
//! Per stream, the opener sends one JSON line, then a body; the other side answers with one JSON
//! line ({"status": ...}), then a body.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const ALPN: &[u8] = b"tooob/0";

/// What a hello signs with the Visitor Key: the protocol, the Host, and the one connection it
/// vouches for (so a copied hello is useless on any other connection).
pub fn hello_message(host_key: &iroh::EndpointId, connection: &iroh::EndpointId) -> Vec<u8> {
    [
        b"tooob/0 hello".as_slice(),
        host_key.as_bytes(),
        connection.as_bytes(),
    ]
    .concat()
}
const MAX_HEADER_LINE: usize = 64 * 1024;

/// The headers in a request's JSON line: `[["name", "value"], ...]`.
pub(crate) fn request_headers(head: &Value) -> impl Iterator<Item = (&str, &str)> {
    head["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| Some((h[0].as_str()?, h[1].as_str()?)))
}

/// A response's JSON line: its status, and the headers `keep` lets through.
pub(crate) fn response_head(
    status: http::StatusCode,
    headers: &http::HeaderMap,
    keep: impl Fn(&str) -> bool,
) -> Value {
    let headers: Vec<_> = headers
        .iter()
        .filter(|(k, _)| keep(k.as_str()))
        .map(|(k, v)| json!([k.as_str(), String::from_utf8_lossy(v.as_bytes())]))
        .collect();
    json!({"status": status.as_u16(), "headers": headers})
}

/// Read up to the first newline; return the parsed JSON and any bytes after it.
pub async fn read_line(recv: &mut (impl AsyncRead + Unpin)) -> Result<(Value, Vec<u8>)> {
    let mut buf = Vec::new();
    match next_line(recv, &mut buf).await? {
        Some(line) => Ok((line, buf)),
        None => bail!("stream ended before header"),
    }
}

/// The next JSON line, leaving what follows it in `buf`; None at end of stream. Cancel-safe
/// (partial input stays in `buf`), so it can wait in a `select!`.
pub async fn next_line(
    recv: &mut (impl AsyncRead + Unpin),
    buf: &mut Vec<u8>,
) -> Result<Option<Value>> {
    loop {
        if let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let line = serde_json::from_slice(&buf[..i]);
            buf.drain(..=i);
            return Ok(Some(line?));
        }
        // Read before the Allowlist check, so a stranger can make us hold this much per stream:
        // keep it near real header sizes (browsers send well under 16 KiB).
        if buf.len() > MAX_HEADER_LINE {
            bail!("header line too long");
        }
        if recv.read_buf(buf).await? == 0 {
            return Ok(None);
        }
    }
}

pub async fn write_line(send: &mut (impl AsyncWrite + Unpin), v: &Value) -> Result<()> {
    let mut line = serde_json::to_vec(v)?;
    line.push(b'\n');
    send.write_all(&line).await?;
    Ok(())
}

/// WebSocket messages over a tooob stream: [kind u8: 0 text, 1 binary, 2 close][len u32 BE][payload].
pub const TEXT: u8 = 0;
pub const BINARY: u8 = 1;
pub const CLOSE: u8 = 2;

pub fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(5 + payload.len());
    f.push(kind);
    f.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    f.extend_from_slice(payload);
    f
}

/// Take one whole frame off the front of `buf`, if one is there: (kind, payload).
pub fn take_frame(buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    let len = u32::from_be_bytes(buf.get(1..5)?.try_into().ok()?) as usize;
    let payload = buf.get(5..5 + len)?.to_vec();
    let kind = buf[0];
    buf.drain(..5 + len);
    Some((kind, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_come_back_whole_and_partial_ones_wait() {
        let mut buf = frame(TEXT, b"hi");
        buf.extend(frame(BINARY, &[9; 3]));
        buf.push(CLOSE); // start of a third frame
        assert_eq!(take_frame(&mut buf), Some((TEXT, b"hi".to_vec())));
        assert_eq!(take_frame(&mut buf), Some((BINARY, vec![9; 3])));
        assert_eq!(
            take_frame(&mut buf),
            None,
            "a partial frame waits for more bytes"
        );
        assert_eq!(buf, vec![CLOSE]);
    }
}
