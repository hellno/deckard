//! Live smoke test: resolve an ENS name and read its real portfolio over the default
//! public RPC, end to end through the same `EthProvider` the GUI uses. Run with:
//!   cargo run -p deckard-core --example smoke
//! Prints the resolved address, native ETH, and each non-zero listed-token balance.

use deckard_core::{format_amount, EthProvider, DEFAULT_RPC};

fn main() {
    let eth = EthProvider::spawn(DEFAULT_RPC);

    let name = "vitalik.eth";
    let addr = match eth.resolve_name(name).recv() {
        Ok(Ok(a)) => a,
        other => {
            eprintln!("resolve {name} failed: {other:?}");
            std::process::exit(1);
        }
    };
    println!("{name} -> {addr}");

    match eth.portfolio(addr).recv() {
        Ok(Ok(read)) => {
            // The trust label the read carries (Helios-Verified vs Unsynced).
            println!("read status: {}", read.status);
            let p = read.value;
            println!("ETH: {}", format_amount(p.native_wei, 18, 6));
            for t in &p.tokens {
                println!("{:>5}: {}", t.symbol, format_amount(t.raw, t.decimals, 4));
            }
        }
        other => {
            eprintln!("portfolio failed: {other:?}");
            std::process::exit(1);
        }
    }
}
