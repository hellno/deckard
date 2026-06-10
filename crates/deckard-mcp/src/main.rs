//! `deckard-mcp` entry point: secret-flag scan first (before clap ever parses), then either
//! the MCP stdio server (`--mcp`) or the CLI command tree — both thin shells over the same
//! key-less [`deckard_mcp::Sidecar`], so nothing is reachable only via Claude.

use clap::{Parser, Subcommand};

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

    let sidecar = Sidecar::from_env()?;
    let result = match &command {
        Command::Balance => sidecar.wallet_balance().await,
        Command::Address => sidecar.wallet_address().await,
        Command::Policy => sidecar.policy_get().await,
        Command::Shield { amount_eth } => sidecar.shield(amount_eth).await,
        Command::Execute { request_id } => sidecar.execute(request_id).await,
        Command::Stop => sidecar.revoke_all().await,
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
