# claude-analyzer

Where did your Claude Code tokens go? Reads the transcripts already on your
machine and opens an HTML report for the last 7 days. Nothing leaves your
computer.

## Run it

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/CaelTC/dotfiles/main/claude-analyzer/install.ps1 | iex
```

**macOS / Linux** — no prebuilt binary yet; build from source:

```sh
cargo run --release
```

Pass a path (`cargo run --release -- report.html`) to write the report without
opening a browser.

See [CONTEXT.md](CONTEXT.md) for what the report's numbers mean.
