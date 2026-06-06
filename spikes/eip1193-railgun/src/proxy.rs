//! A tiny HTTP/1.1 reverse proxy that **records every JSON-RPC `method`** it
//! forwards, then passes the request through unchanged to Helios's localhost
//! JSON-RPC server.
//!
//! This is Task 4 of the spike: instrument the seam so we can *empirically*
//! enumerate exactly which methods the alloy provider (and, under `--features
//! railgun`, the real Railgun read/sync path) invokes — and cross-check each
//! against the set Helios's localhost server actually serves.
//!
//! Shape (same loopback hop v1 uses, plus a logging tap):
//!   alloy provider ──HTTP──▶ this proxy (logs `method`) ──HTTP──▶ Helios :H
//!
//! Adapted from `spikes/helios-walkaway/src/proxy.rs` (the killable/lying proxy),
//! with the cut/lie machinery removed and method-logging added.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use eyre::{eyre, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Shared record of the JSON-RPC methods seen, in first-seen order, with counts.
#[derive(Clone, Default)]
pub struct MethodLog {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// method -> call count
    counts: BTreeMap<String, u64>,
    /// methods in first-seen order
    order: Vec<String>,
}

impl MethodLog {
    pub fn record(&self, method: &str) {
        let mut g = self.inner.lock().unwrap();
        if !g.counts.contains_key(method) {
            g.order.push(method.to_string());
        }
        *g.counts.entry(method.to_string()).or_insert(0) += 1;
    }

    /// (method, count) pairs in first-seen order.
    pub fn snapshot(&self) -> Vec<(String, u64)> {
        let g = self.inner.lock().unwrap();
        g.order
            .iter()
            .map(|m| (m.clone(), *g.counts.get(m).unwrap_or(&0)))
            .collect()
    }
}

/// Bind a logging proxy on `bind_addr`, forwarding JSON-RPC POSTs to `upstream`
/// (Helios's localhost server). Returns the shared [`MethodLog`]. Returns once
/// the listener is bound so the caller can immediately point a provider at it.
pub async fn spawn(bind_addr: &str, upstream: String) -> Result<MethodLog> {
    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|e| eyre!("proxy bind {bind_addr} failed: {e}"))?;
    let log = MethodLog::default();

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let log_for_task = log.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((sock, _peer)) => {
                    let http = http.clone();
                    let upstream = upstream.clone();
                    let log = log_for_task.clone();
                    tokio::spawn(async move {
                        let _ = handle_conn(sock, http, upstream, log).await;
                    });
                }
                Err(_) => break,
            }
        }
    });

    Ok(log)
}

async fn handle_conn(
    mut sock: TcpStream,
    http: reqwest::Client,
    upstream: String,
    log: MethodLog,
) -> Result<()> {
    loop {
        let body = match read_request_body(&mut sock).await? {
            Some(b) => b,
            None => return Ok(()), // peer closed cleanly
        };

        // Record the method(s) — JSON-RPC requests are a single object or a batch array.
        record_methods(&body, &log);

        let resp = http
            .post(&upstream)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await;

        let out = match resp {
            Ok(r) => r.bytes().await.unwrap_or_default().to_vec(),
            Err(_) => {
                let _ = sock.shutdown().await;
                return Ok(());
            }
        };

        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            out.len()
        );
        sock.write_all(head.as_bytes()).await?;
        sock.write_all(&out).await?;
        sock.flush().await?;
    }
}

/// Parse a JSON-RPC request body and record each `method` into the log.
fn record_methods(body: &[u8], log: &MethodLog) {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return;
    };
    match v {
        serde_json::Value::Array(batch) => {
            for item in batch {
                if let Some(m) = item.get("method").and_then(|m| m.as_str()) {
                    log.record(m);
                }
            }
        }
        obj => {
            if let Some(m) = obj.get("method").and_then(|m| m.as_str()) {
                log.record(m);
            }
        }
    }
}

/// Minimal HTTP/1.1 request reader: read headers, parse `Content-Length`, read
/// exactly that many body bytes. alloy's reqwest client always sends
/// Content-Length JSON POSTs (never chunked), so this is sufficient. Returns
/// `None` if the peer closed before sending anything.
async fn read_request_body(sock: &mut TcpStream) -> Result<Option<Vec<u8>>> {
    let mut buf = Vec::with_capacity(2048);
    let mut tmp = [0u8; 2048];

    let header_end = loop {
        if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        let n = sock.read(&mut tmp).await?;
        if n == 0 {
            return Ok(if buf.is_empty() { None } else { Some(Vec::new()) });
        }
        buf.extend_from_slice(&tmp[..n]);
    };

    let content_length = parse_content_length(&buf[..header_end]).unwrap_or(0);
    while buf.len() < header_end + content_length {
        let n = sock.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }

    Ok(Some(
        buf[header_end..(header_end + content_length).min(buf.len())].to_vec(),
    ))
}

fn parse_content_length(header_bytes: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(header_bytes);
    for line in text.split("\r\n") {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                return v.trim().parse::<usize>().ok();
            }
        }
    }
    None
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
