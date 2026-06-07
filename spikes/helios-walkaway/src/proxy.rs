//! A tiny, *killable* — and optionally *lying* — HTTP/1.1 reverse proxy.
//!
//! Two roles, both used to demonstrate what embedding Helios actually buys:
//!
//! 1. **Killable** ("cut the centralized RPC on camera"): Helios's primary EL is
//!    pointed at `http://127.0.0.1:<port>` instead of straight at the upstream;
//!    flipping the kill switch makes every request to that port fail at the
//!    transport layer — a yanked cable / revoked key / firewall rule. (Availability.)
//!
//! 2. **Lying** (`lie = true`): the proxy tampers the `balance` field of every
//!    `eth_getProof` response before returning it. This is a malicious/compromised
//!    RPC. Helios rebuilds the account from that tampered balance, RLP-encodes it,
//!    and checks it against the Merkle proof under the CL-signed state root — the
//!    check **fails** (`InvalidAccountProof`) and the read is **refused**. A
//!    centralized wallet would just display the lie. (Integrity — the real moat.)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eyre::{eyre, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Flip `.cut` to true to sever the proxied endpoint.
#[derive(Clone)]
pub struct KillSwitch {
    cut: Arc<AtomicBool>,
}

impl KillSwitch {
    pub fn cut(&self) {
        self.cut.store(true, Ordering::SeqCst);
    }
    #[allow(dead_code)]
    pub fn is_cut(&self) -> bool {
        self.cut.load(Ordering::SeqCst)
    }
}

/// Bind a proxy on `bind_addr` forwarding JSON-RPC POSTs to `upstream`. If `lie`,
/// it tampers the balance in every `eth_getProof` response (a malicious RPC).
/// Returns once the listener is bound, so the caller can immediately build a Helios
/// client against the local address.
pub async fn spawn(bind_addr: &str, upstream: String, lie: bool) -> Result<KillSwitch> {
    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|e| eyre!("proxy bind {bind_addr} failed: {e}"))?;
    let cut = Arc::new(AtomicBool::new(false));
    let switch = KillSwitch { cut: cut.clone() };

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((sock, _peer)) => {
                    let cut = cut.clone();
                    let http = http.clone();
                    let upstream = upstream.clone();
                    tokio::spawn(async move {
                        let _ = handle_conn(sock, cut, http, upstream, lie).await;
                    });
                }
                Err(_) => break,
            }
        }
    });

    Ok(switch)
}

async fn handle_conn(
    mut sock: TcpStream,
    cut: Arc<AtomicBool>,
    http: reqwest::Client,
    upstream: String,
    lie: bool,
) -> Result<()> {
    loop {
        // Read one HTTP/1.1 request (headers + Content-Length body).
        let req = match read_request(&mut sock).await? {
            Some(r) => r,
            None => return Ok(()), // peer closed cleanly
        };

        // THE CUT: once severed, drop every request at the transport layer.
        if cut.load(Ordering::SeqCst) {
            let _ = sock.shutdown().await;
            return Ok(());
        }

        let is_get_proof = lie && find_subsequence(&req.body, b"eth_getProof").is_some();

        let resp = http
            .post(&upstream)
            .header("content-type", "application/json")
            .body(req.body)
            .send()
            .await;

        let mut body = match resp {
            Ok(r) => r.bytes().await.unwrap_or_default().to_vec(),
            // Upstream itself erred — surface a transport close so the client fails over.
            Err(_) => {
                let _ = sock.shutdown().await;
                return Ok(());
            }
        };

        // THE LIE: tamper the proven balance. Helios will reject it.
        if is_get_proof {
            if let Some(tampered) = tamper_balance(&body) {
                body = tampered;
            }
        }

        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        );
        sock.write_all(head.as_bytes()).await?;
        sock.write_all(&body).await?;
        sock.flush().await?;
    }
}

struct ParsedRequest {
    body: Vec<u8>,
}

/// Minimal HTTP/1.1 request reader: reads until the header terminator, parses
/// `Content-Length`, then reads exactly that many body bytes. Helios's reqwest
/// client always sends Content-Length JSON POSTs (never chunked), so this is
/// sufficient. Returns `None` if the peer closed before sending anything.
async fn read_request(sock: &mut TcpStream) -> Result<Option<ParsedRequest>> {
    let mut buf = Vec::with_capacity(2048);
    let mut tmp = [0u8; 2048];

    // Read until we have the full header block.
    let header_end = loop {
        if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        let n = sock.read(&mut tmp).await?;
        if n == 0 {
            return Ok(if buf.is_empty() { None } else { Some(ParsedRequest { body: Vec::new() }) });
        }
        buf.extend_from_slice(&tmp[..n]);
    };

    let content_length = parse_content_length(&buf[..header_end]).unwrap_or(0);

    // Read the remaining body bytes.
    while buf.len() < header_end + content_length {
        let n = sock.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }

    let body = buf[header_end..(header_end + content_length).min(buf.len())].to_vec();
    Ok(Some(ParsedRequest { body }))
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
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// The bogus balance a malicious RPC claims: 1,000,000,000 ETH (1e27 wei).
pub const LIE_BALANCE_HEX: &str = "0x33b2e3c9fd0803ce8000000";

/// Rewrite `result.balance` in an `eth_getProof` JSON response to [`LIE_BALANCE_HEX`],
/// leaving the (real) Merkle proof untouched — so Helios's proof check will reject it.
fn tamper_balance(body: &[u8]) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let bal = v.get_mut("result")?.get_mut("balance")?;
    if !bal.is_string() {
        return None;
    }
    *bal = serde_json::Value::String(LIE_BALANCE_HEX.to_string());
    serde_json::to_vec(&v).ok()
}
