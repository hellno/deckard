//! A tiny blocking HTTP client to the browser bridge's `/admin` endpoint (#199).
//!
//! The app lists and disconnects dapp sessions by POSTing to the bridge's loopback HTTP server —
//! the same server that already serves `/rpc` — so there is ZERO daemon / wire / contract change
//! and no new dependency (this is a hand-rolled [`std::net::TcpStream`] client, not an HTTP crate).
//!
//! **Blocking, background-only.** Every function here does synchronous socket IO, so it MUST only
//! ever be called from inside `cx.background_spawn(...)`, never on the GPUI UI thread — a dead or
//! slow bridge would otherwise stall the frame. The connect/read/write timeouts below bound a hung
//! listener; a bridge that isn't running fails instantly with `ECONNREFUSED`.
//!
//! Revoke is a session-scoped DISCONNECT, not a ban (ADR 0006): a revoked site's next request
//! fails `4100 Unauthorized`, and it may reconnect for a fresh session.
//!
//! **Error semantics.** Sessions live only in the bridge's RAM, so a bridge that isn't listening
//! genuinely means no live sessions: a failed CONNECT resolves to an honest empty list. But a
//! reachable-but-unhealthy bridge (a read/write timeout, a partial response, a non-200) resolves to
//! `Err` — NOT an empty list — so the caller keeps its current view rather than falsely blanking
//! live sessions on a momentary hiccup. A transient error is not proof a dapp disconnected.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::Deserialize;

/// Environment override for the bridge's loopback address; defaults to the bridge/justfile default
/// (`deckard-browser-bridge --bind`, `just qa-bridge`/`demo-bridge` both use `127.0.0.1:8765`).
const BRIDGE_ADDR_ENV: &str = "DECKARD_BRIDGE_ADDR";
const DEFAULT_BRIDGE_ADDR: &str = "127.0.0.1:8765";

/// How long to wait for the loopback TCP connect. A live bridge accepts instantly; a dead port
/// refuses instantly (`ECONNREFUSED`). This only bounds a wedged listener.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
/// Read/write timeouts once connected, so a hung bridge can't stall the background thread forever.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// One connected dapp site, as the Connections sidebar renders it (#199). Mirrors the bridge's live
/// `DappSession`, minus the fields the app doesn't display; serde ignores the extra `revoked` /
/// `permissions` keys the bridge still serializes (no `deny_unknown_fields`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ConnectedSite {
    pub origin: String,
    pub account: String,
    pub chain_id: u64,
    pub created_at: u64,
    pub last_seen: u64,
}

/// The bridge address to dial: `DECKARD_BRIDGE_ADDR` or the default loopback port.
fn bridge_addr() -> String {
    std::env::var(BRIDGE_ADDR_ENV).unwrap_or_else(|_| DEFAULT_BRIDGE_ADDR.to_string())
}

/// POST one admin op body to `/admin` and parse the returned live session list. Blocking;
/// background-only. Writes a minimal HTTP/1.1 request with the `x-deckard-admin` marker header the
/// bridge gates on (a web page can't set it — the CORS preflight forbids it), then reads to EOF
/// (the bridge answers `connection: close`).
/// Resolve the bridge address to a concrete LOOPBACK socket, or an error. Refuses a non-loopback
/// target: the bridge only ever binds loopback (`serve` bails otherwise), so a non-loopback
/// `DECKARD_BRIDGE_ADDR` is a misconfiguration — we must never write the `x-deckard-admin` marker +
/// a disconnect command to a remote host, nor trust its session list. Mirrors the server's own
/// loopback guard; the marker header must not leave this machine. Pure w.r.t. env (takes the addr
/// string) so it's unit-testable without a socket.
fn resolve_loopback_addr(addr: &str) -> anyhow::Result<std::net::SocketAddr> {
    let socket_addr = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("bridge address {addr} did not resolve"))?;
    if !socket_addr.ip().is_loopback() {
        anyhow::bail!("refusing admin request to non-loopback bridge address {addr}");
    }
    Ok(socket_addr)
}

fn admin_request(body: &str) -> anyhow::Result<Vec<ConnectedSite>> {
    let addr = bridge_addr();
    let socket_addr = resolve_loopback_addr(&addr)?;
    let mut stream = match TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT) {
        Ok(stream) => stream,
        // REFUSED is the one connect failure that proves nothing is listening: there is no bridge,
        // so there are no live sessions — they live in the bridge's RAM. Report an honest empty
        // list. Every OTHER failure (a connect timeout on a momentarily-stalled accept loop, an
        // unreachable error) is NOT proof the bridge is gone, so it propagates as `Err` — same as
        // a post-connect failure — and the caller keeps its current list instead of falsely
        // blanking still-live sessions.
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            return Ok(Vec::new())
        }
        Err(error) => return Err(error.into()),
    };
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let request = format!(
        "POST /admin HTTP/1.1\r\nhost: {addr}\r\nx-deckard-admin: 1\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    parse_admin_response(&raw)
}

/// Parse a bridge `/admin` HTTP response into the live session list. PURE (no IO) so it's unit-
/// testable with canned bytes: require `200 OK` on the status line, split headers from body on the
/// blank line, and read `{"sessions":[…]}`. Any non-200 is an error the caller maps to "no live
/// sessions" — the honest answer when the bridge refuses or is unhealthy.
pub fn parse_admin_response(raw: &[u8]) -> anyhow::Result<Vec<ConnectedSite>> {
    let text = std::str::from_utf8(raw)?;
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("bridge response has no header/body separator"))?;
    let status_line = head
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("bridge response has no status line"))?;
    // `HTTP/1.1 200 OK` — the status code is the second whitespace-separated token.
    let ok = status_line
        .split_whitespace()
        .nth(1)
        .is_some_and(|code| code == "200");
    if !ok {
        anyhow::bail!("bridge admin request failed: {status_line}");
    }

    #[derive(Deserialize)]
    struct AdminResponse {
        sessions: Vec<ConnectedSite>,
    }
    let parsed: AdminResponse = serde_json::from_str(body.trim())?;
    Ok(parsed.sessions)
}

/// List the bridge's live (non-revoked) dapp sessions. Blocking; background-only.
pub fn list_sites() -> anyhow::Result<Vec<ConnectedSite>> {
    admin_request(r#"{"op":"list"}"#)
}

/// Disconnect one site (session-scoped, NOT a ban — ADR 0006) and return the remaining live list.
/// Blocking; background-only. Revoking an unknown/already-gone origin is a no-op on the bridge.
pub fn revoke_site(origin: &str) -> anyhow::Result<Vec<ConnectedSite>> {
    let body = serde_json::json!({ "op": "revoke", "origin": origin }).to_string();
    admin_request(&body)
}

/// Disconnect every live site (Lock / Lockdown reduce all authority) and return the now-empty list.
/// Blocking; background-only.
pub fn clear_sites() -> anyhow::Result<Vec<ConnectedSite>> {
    admin_request(r#"{"op":"clear"}"#)
}

/// The host portion of a dapp origin for sidebar display: strips a leading `http://` / `https://`
/// so `https://app.uniswap.org` reads as `app.uniswap.org`. Anything without one of those schemes
/// (the bridge's `unknown-origin` fallback, or an unusual scheme) passes through unchanged — we
/// never fabricate or hide what the origin actually is.
pub fn origin_host(origin: &str) -> &str {
    origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a canned HTTP response so the pure parser can be exercised without a socket.
    fn http(status_line: &str, body: &str) -> Vec<u8> {
        format!(
            "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn parse_admin_response_reads_sessions_on_200() {
        let body = r#"{"sessions":[{"origin":"https://app.example","chain_id":11155111,"account":"0xabc","permissions":["eth_accounts"],"created_at":1,"last_seen":2,"revoked":false}]}"#;
        let raw = http("HTTP/1.1 200 OK", body);
        let sites = parse_admin_response(&raw).expect("200 parses");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].origin, "https://app.example");
        assert_eq!(sites[0].account, "0xabc");
        assert_eq!(sites[0].chain_id, 11155111);
        assert_eq!(sites[0].created_at, 1);
        assert_eq!(sites[0].last_seen, 2);
    }

    #[test]
    fn parse_admin_response_reads_empty_session_list() {
        let raw = http("HTTP/1.1 200 OK", r#"{"sessions":[]}"#);
        assert!(parse_admin_response(&raw).expect("empty parses").is_empty());
    }

    #[test]
    fn parse_admin_response_rejects_non_200() {
        // A 400/403 must NOT masquerade as an empty session list — it's an error the caller maps to
        // "no live sessions" itself, distinctly from a real empty 200.
        let raw = http("HTTP/1.1 400 Bad Request", "unknown admin op");
        assert!(parse_admin_response(&raw).is_err());
    }

    #[test]
    fn parse_admin_response_rejects_garbage() {
        assert!(parse_admin_response(b"not an http response at all").is_err());
        // A 200 with a non-JSON body is still an error.
        let raw = http("HTTP/1.1 200 OK", "<html>nope</html>");
        assert!(parse_admin_response(&raw).is_err());
    }

    #[test]
    fn resolve_loopback_addr_accepts_loopback_and_refuses_the_rest() {
        // Loopback literals resolve without DNS and pass. Note [::1] passes THIS guard (its job is
        // only "the admin marker never leaves this machine") but today's bridge can't bind or serve
        // it (`serve`/`host_is_loopback` accept 127.0.0.1/localhost only), so an IPv6 override
        // fails CLOSED downstream with a 403 — accepted here for the guard's own honesty, not as
        // an end-to-end capability claim.
        assert!(resolve_loopback_addr("127.0.0.1:8765").is_ok());
        assert!(resolve_loopback_addr("[::1]:8765").is_ok());
        // A non-loopback literal is refused — the admin marker must never leave this machine, even
        // if DECKARD_BRIDGE_ADDR is misconfigured to a routable host. Literal IPs, so no DNS.
        assert!(resolve_loopback_addr("8.8.8.8:80").is_err());
        assert!(resolve_loopback_addr("93.184.216.34:80").is_err());
        // Malformed / unresolvable input errors rather than dialing anything.
        assert!(resolve_loopback_addr("not-an-address").is_err());
        assert!(resolve_loopback_addr("127.0.0.1").is_err());
    }

    #[test]
    fn origin_host_strips_scheme_and_passes_unknown_through() {
        assert_eq!(origin_host("https://app.uniswap.org"), "app.uniswap.org");
        assert_eq!(origin_host("http://127.0.0.1:8777"), "127.0.0.1:8777");
        // The bridge's fallback and any non-web scheme pass through unchanged — never disguised.
        assert_eq!(origin_host("unknown-origin"), "unknown-origin");
        assert_eq!(origin_host("app.example.org"), "app.example.org");
    }
}
