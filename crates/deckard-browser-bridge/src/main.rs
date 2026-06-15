//! `deckard-browser-bridge` entry point: the key-less dapp/browser interface.

use clap::Parser;
use deckard_wallet_client::WalletClient;

#[derive(Parser)]
#[command(
    name = "deckard-browser-bridge",
    version,
    about = "Deckard's experimental key-less EIP-1193 localhost browser bridge. Holds no keys."
)]
struct Cli {
    /// Loopback bind address. Keep this on 127.0.0.1 unless you are debugging.
    #[arg(long, default_value = "127.0.0.1:8765")]
    bind: String,
    /// Return this mock address instead of reading an unlocked Deckard daemon.
    #[arg(long = "dev-mock-account")]
    dev_mock_account: Option<String>,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
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
    let wallet = WalletClient::from_env()?;
    deckard_browser_bridge::serve(&cli.bind, wallet, cli.dev_mock_account).await
}
