//! `claude-dash` — a live terminal dashboard for watching consumption of a
//! Claude subscription across concurrent **Session**s.
//!
//! Two modes:
//! - `claude-dash` (no args) — the read-only TUI rendering the **Budget** rail.
//! - `claude-dash proxy` — the streaming reverse-**Proxy** that captures
//!   **Budget** from `anthropic-ratelimit-unified-*` headers.

mod budget;
mod proxy;
mod record;
mod store;
mod tui;

use std::net::SocketAddr;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// `claude-dash` — Budget/Throughput dashboard over the local capture **Proxy**.
#[derive(Parser, Debug)]
#[command(name = "claude-dash", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the streaming reverse-**Proxy** the client points `ANTHROPIC_BASE_URL` at.
    Proxy {
        /// Address to listen on. `0` for the port picks an ephemeral one.
        #[arg(long, default_value = "127.0.0.1:8787")]
        addr: SocketAddr,

        /// **Session** id (store key). Self-generated if omitted.
        #[arg(long)]
        id: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Proxy { addr, id }) => {
            // The Proxy is async; the TUI is sync. Spin a runtime only here.
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(proxy::run(addr, id))
        }
        None => tui::run(),
    }
}
