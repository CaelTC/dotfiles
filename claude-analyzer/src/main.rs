//! No arguments: build this Week's report and open it. One argument: write the
//! report there and stay out of the browser.

use chrono::Utc;
use std::path::PathBuf;
use std::process::Command;

const TEMPLATE: &str = include_str!("../assets/report.html");
const TOKEN: &str = "__CLAUDE_ANALYZER_DATA__";

fn main() {
    let out = std::env::args().nth(1);
    let root = claude_analyzer::default_root();
    if !root.is_dir() {
        eprintln!("no transcripts found at {}", root.display());
        std::process::exit(1);
    }
    let data = claude_analyzer::analyze(&root, Utc::now());
    let html = TEMPLATE.replace(TOKEN, &data.to_string());

    let path = out.clone().map(PathBuf::from).unwrap_or_else(|| {
        std::env::temp_dir().join("claude-analyzer-report.html")
    });
    if let Err(e) = std::fs::write(&path, html) {
        eprintln!("could not write {}: {e}", path.display());
        std::process::exit(1);
    }
    println!(
        "{} — {} requests, {} fresh tokens across {} sessions",
        path.display(),
        data["totals"]["requests"],
        data["totals"]["fresh"],
        data["totals"]["sessions"],
    );
    if out.is_none() {
        open(&path);
    }
}

fn open(path: &std::path::Path) {
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/c", "start", ""])
    } else {
        ("xdg-open", vec![])
    };
    if let Err(e) = Command::new(cmd).args(args).arg(path).spawn() {
        eprintln!("open the report yourself: {} ({e})", path.display());
    }
}
