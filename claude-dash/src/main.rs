//! `claude-dash` — a live terminal dashboard for watching consumption of a
//! Claude subscription across concurrent **Session**s.
//!
//! Modes:
//! - `claude-dash` (no args) — the read-only TUI rendering the **Budget** rail
//!   and the N **Active Session** panels.
//! - `claude-dash proxy` — the streaming reverse-**Proxy** that captures
//!   **Budget** from `anthropic-ratelimit-unified-*` headers.
//! - `claude-dash record-start` — append a **Session**'s `start` record; the
//!   `cca` wrapper calls this so the JSONL schema stays owned by the Rust code.
//! - `claude-dash record-end` — append a **Session**'s `end` record when
//!   `claude` exits; `cca` calls this so the schema stays Rust-owned and the
//!   **Session** moves into **Session History**.
//! - `claude-dash status` — a one-shot SwiftBar readout of the current **Budget**
//!   from the store, for a macOS menu-bar **Utilization** %.

mod budget;
mod lifecycle;
mod proxy;
mod record;
mod status;
mod store;
mod throughput;
mod tui;

use std::net::SocketAddr;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::record::{EndRecord, Record, StartRecord};

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

    /// Append a **Session**'s `start` record to its store file. Invoked by the
    /// `cca` wrapper so the JSONL record shape stays owned by the Rust code.
    RecordStart {
        /// The minted **Session** id (store key, JSONL file stem).
        #[arg(long)]
        id: String,

        /// The **Session**'s project — the cwd basename — shown in the panel label.
        #[arg(long)]
        project: String,

        /// The absolute working directory `cca` was launched from.
        #[arg(long)]
        cwd: String,

        /// The launching process id (the **Session**'s liveness handle).
        #[arg(long)]
        pid: i32,
    },

    /// Append a **Session**'s `end` record to its store file when `claude`
    /// exits. Invoked by the `cca` wrapper so the JSONL record shape stays owned
    /// by the Rust code; this is what moves the **Session** into **Session
    /// History**.
    RecordEnd {
        /// The minted **Session** id (store key, JSONL file stem).
        #[arg(long)]
        id: String,
    },

    /// Print the current **Budget** as SwiftBar menu-bar output (title +
    /// dropdown) from the store, then exit 0. Fed by a SwiftBar plugin so a macOS
    /// menu-bar item shows the **Representative Window**'s **Utilization** %.
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Proxy { addr, id }) => {
            // The Proxy is async; the TUI is sync. Spin a runtime only here.
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(proxy::run(addr, id))
        }
        Some(Command::RecordStart {
            id,
            project,
            cwd,
            pid,
        }) => {
            // cca mints the id and learns the pid; we own the record shape and
            // the store path so the JSONL schema lives in one place.
            let dir = store::sessions_dir()?;
            let path = store::session_path(&dir, &id);
            let record = Record::Start(StartRecord {
                id,
                ts: chrono::Utc::now().timestamp_millis(),
                project,
                cwd,
                pid,
            });
            store::append_record(&path, &record)
        }
        Some(Command::RecordEnd { id }) => {
            // cca calls this when `claude` exits. The `end` ts is the Session's
            // end time; the classifier reads it to move the Session into Session
            // History. We own the record shape and store path.
            let dir = store::sessions_dir()?;
            let path = store::session_path(&dir, &id);
            let record = Record::End(EndRecord {
                id,
                ts: chrono::Utc::now().timestamp_millis(),
            });
            store::append_record(&path, &record)
        }
        Some(Command::Status) => status::run(),
        None => tui::run(),
    }
}
