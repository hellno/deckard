//! `deckard-mcp` entry point: secret-flag scan first (before clap ever parses), then either
//! the MCP stdio server (`--mcp`) or the CLI command tree — both thin shells over the same
//! key-less [`deckard_mcp::Sidecar`], so nothing is reachable only via Claude.

use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use deckard_mcp::browser_bridge::{
    dev_account_env_name, BridgeBackend, BridgeRequest, BrowserBridge,
};
use deckard_mcp::{install, secrets, server, Sidecar};

#[derive(Parser)]
#[command(
    name = "deckard-mcp",
    version,
    about = "Deckard's key-less agent surface: CLI + MCP stdio server (mcp.v0.1, 6 tools). \
             Holds no keys — every write is proposed to the local deckard-signerd, which \
             enforces policy and signs.",
    after_help = "Env: DECKARD_SOCKET_PATH (daemon socket), DECKARD_CHAIN_ID (default 1), \
                  DECKARD_CONFIG_DIR (sharpens 'locked' vs 'no wallet' errors). \
                  `install --demo` prints the demo block for Claude Desktop. \
                  Secrets are never accepted on this command line."
)]
struct Cli {
    /// Run as an MCP stdio server (the Claude Desktop registration target).
    #[arg(long)]
    mcp: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Read the public balance through the daemon (shielded shows in the Deckard app).
    Balance,
    /// Read the wallet's public address through the daemon.
    Address,
    /// Read the signing policy fence (caps, approval mode, STOP state).
    Policy,
    /// Propose shielding ETH into the wallet's own private balance, then print the
    /// request id to pass to `execute`.
    Shield {
        /// Amount as a decimal ETH string, e.g. 0.02 (units: ETH, not wei).
        #[arg(long = "amount-eth")]
        amount_eth: String,
    },
    /// Sign + broadcast a previously-proposed request id. If this times out, the status
    /// is UNKNOWN — do not re-run it; check the Deckard app.
    Execute {
        /// The 0x-hex request id returned by `shield`.
        request_id: String,
    },
    /// STOP — the panic brake: zeroize the key, lock the daemon, deny everything
    /// in flight. Re-arm by unlocking in the Deckard app.
    Stop,
    /// Experimental localhost EIP-1193 browser bridge for the unpacked extension.
    BrowserBridge {
        /// Loopback bind address. Keep this on 127.0.0.1 unless you are debugging.
        #[arg(long, default_value = "127.0.0.1:8765")]
        bind: String,
        /// Return this mock address instead of reading an unlocked Deckard daemon.
        #[arg(long = "dev-mock-account")]
        dev_mock_account: Option<String>,
    },
    /// Print (or, with --write + confirmation, write) the Claude Desktop registration.
    Install {
        /// Emit the demo env block (isolated config dir + socket, Sepolia fork chain id,
        /// local anvil RPC) instead of targeting the everyday daemon.
        #[arg(long)]
        demo: bool,
        /// Merge the entry into claude_desktop_config.json (asks for confirmation).
        #[arg(long)]
        write: bool,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // Hard-reject secret-shaped flags BEFORE clap parses (and without echoing values).
    if let Err(msg) = secrets::reject_secret_flags(std::env::args().skip(1)) {
        eprintln!("{msg}");
        return std::process::ExitCode::from(2);
    }

    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.mcp {
        if cli.command.is_some() {
            anyhow::bail!("--mcp runs the stdio server; combine it with no subcommand");
        }
        return server::serve_stdio(Sidecar::from_env()?).await;
    }

    let Some(command) = cli.command else {
        anyhow::bail!(
            "nothing to do — pass --mcp to run the MCP server, or a subcommand \
             (try: deckard-mcp --help)"
        );
    };

    // Install needs no daemon; everything else talks through the sidecar.
    if let Command::Install { demo, write } = command {
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        return install::run(demo, write, &mut lock);
    }

    if let Command::BrowserBridge {
        bind,
        dev_mock_account,
    } = &command
    {
        let sidecar = Sidecar::from_env()?;
        return serve_browser_bridge(bind, sidecar, dev_mock_account.clone()).await;
    }

    let sidecar = Sidecar::from_env()?;
    let result = match &command {
        Command::Balance => sidecar.wallet_balance().await,
        Command::Address => sidecar.wallet_address().await,
        Command::Policy => sidecar.policy_get().await,
        Command::Shield { amount_eth } => sidecar.shield(amount_eth).await,
        Command::Execute { request_id } => sidecar.execute(request_id).await,
        Command::Stop => sidecar.revoke_all().await,
        Command::BrowserBridge { .. } => unreachable!("handled above"),
        Command::Install { .. } => unreachable!("handled above"),
    };
    match result {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        Err(failure) => {
            anyhow::bail!("{}", failure.to_human())
        }
    }
}

async fn serve_browser_bridge(
    bind: &str,
    sidecar: Sidecar,
    dev_mock_account: Option<String>,
) -> anyhow::Result<()> {
    if !bind.starts_with("127.0.0.1:") && !bind.starts_with("localhost:") {
        anyhow::bail!("browser bridge must bind to loopback (example: 127.0.0.1:8765)");
    }
    let chain_id = sidecar.chain_id();
    let backend = match dev_mock_account {
        Some(account) => BridgeBackend::DevMock { account },
        None => BridgeBackend::from_env(sidecar),
    };
    let bridge = Arc::new(BrowserBridge::new(chain_id, backend));
    let listener = TcpListener::bind(bind).await?;
    eprintln!(
        "Deckard browser bridge listening on http://{bind}/rpc (dev mock via {})",
        dev_account_env_name()
    );
    loop {
        let (stream, _) = listener.accept().await?;
        let bridge = Arc::clone(&bridge);
        tokio::spawn(async move {
            if let Err(e) = handle_http_connection(stream, bridge).await {
                eprintln!("browser bridge request failed: {e}");
            }
        });
    }
}

async fn handle_http_connection(
    mut stream: TcpStream,
    bridge: Arc<BrowserBridge>,
) -> anyhow::Result<()> {
    let mut buf = vec![0_u8; 64 * 1024];
    let mut read = 0_usize;
    let header_end = loop {
        let n = stream.read(&mut buf[read..]).await?;
        if n == 0 {
            return Ok(());
        }
        read += n;
        if let Some(pos) = find_header_end(&buf[..read]) {
            break pos;
        }
        if read == buf.len() {
            return write_http(&mut stream, 413, "text/plain", "request too large").await;
        }
    };

    let headers = std::str::from_utf8(&buf[..header_end])?.to_string();
    let (method, path) = request_line(&headers)?;
    let origin = header_value(&headers, "x-deckard-origin")
        .or_else(|| header_value(&headers, "origin"))
        .unwrap_or("unknown-origin")
        .to_string();

    if method == "OPTIONS" {
        return write_http(&mut stream, 204, "text/plain", "").await;
    }
    if method != "POST" || path != "/rpc" {
        return write_http(&mut stream, 404, "text/plain", "not found").await;
    }
    if !host_is_loopback(&headers) {
        return write_http(&mut stream, 403, "text/plain", "host must be localhost").await;
    }

    let content_length = header_value(&headers, "content-length")
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| anyhow::anyhow!("missing content-length"))?;
    if content_length > 32 * 1024 {
        return write_http(&mut stream, 413, "text/plain", "body too large").await;
    }

    let body_start = header_end + 4;
    while read < body_start + content_length {
        let n = stream.read(&mut buf[read..]).await?;
        if n == 0 {
            anyhow::bail!("connection closed before request body completed");
        }
        read += n;
    }

    let request: BridgeRequest =
        serde_json::from_slice(&buf[body_start..body_start + content_length])?;
    let response = bridge.handle_request(&origin, request).await;
    let response_body = serde_json::to_string(&response)?;
    write_http(&mut stream, 200, "application/json", &response_body).await
}

async fn write_http(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> anyhow::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\naccess-control-allow-origin: *\r\naccess-control-allow-headers: content-type,x-deckard-origin\r\naccess-control-allow-methods: POST,OPTIONS\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn request_line(headers: &str) -> anyhow::Result<(&str, &str)> {
    let line = headers
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing request line"))?;
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing request method"))?;
    let path = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing request path"))?;
    Ok((method, path))
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.eq_ignore_ascii_case(name) {
            Some(value.trim())
        } else {
            None
        }
    })
}

fn host_is_loopback(headers: &str) -> bool {
    header_value(headers, "host")
        .map(|host| host.starts_with("127.0.0.1:") || host.starts_with("localhost:"))
        .unwrap_or(false)
}
