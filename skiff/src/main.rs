// skiff — ferry between your tailscale machines.
//
//   skiff ls                          list machines on the tailnet
//   skiff sessions <host>             list tmux sessions on a machine
//   skiff ssh <host> [session]        ssh in, attach-or-create tmux session (default: main)
//   skiff claude <host> <dir> [-s name] [-- <claude args>]
//                                     start claude in a detached tmux session on <host>
//
// Every entry point lands in a named tmux session on the remote machine, so
// work survives disconnects and you can reattach later — via `skiff ssh` or a
// plain `tmux attach` on the machine itself.

use std::os::unix::process::CommandExt;
use std::process::Command;

use anyhow::{Context, Result, bail};

// Remote non-interactive shells often miss homebrew/cargo paths; tmux and
// claude live there. Prepended to every remote command.
const REMOTE_PATH: &str = r#"export PATH="$PATH:/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.cargo/bin";"#;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ls") => ls(),
        Some("sessions") => sessions(args.get(1).context("usage: skiff sessions <host>")?),
        Some("ssh") => ssh(
            args.get(1).context("usage: skiff ssh <host> [session]")?,
            args.get(2).map(String::as_str).unwrap_or("main"),
        ),
        Some("claude") => claude(&args[1..]),
        _ => {
            eprint!("{}", USAGE);
            std::process::exit(2);
        }
    }
}

const USAGE: &str = "skiff — ferry between your tailscale machines

usage:
  skiff ls                                     list machines on the tailnet
  skiff sessions <host>                        list tmux sessions on <host>
  skiff ssh <host> [session]                   attach-or-create tmux session (default: main)
  skiff claude <host> <dir> [-s name] [-- <claude args>]
                                               start claude in a detached tmux session
                                               (session name defaults to the dir basename)

examples:
  skiff claude work-mac ~/dev/api -- \"fix the failing tests\"
  skiff ssh work-mac api                       # attach to that session later
";

fn tailnet() -> Result<serde_json::Value> {
    let out = Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .context("running tailscale — is it installed?")?;
    if !out.status.success() {
        bail!("tailscale status failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

// The ssh-able machine name is the first label of DNSName (MagicDNS), not HostName.
fn dns_label(node: &serde_json::Value) -> String {
    node["DNSName"]
        .as_str()
        .and_then(|d| d.split('.').next())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| node["HostName"].as_str().unwrap_or("?"))
        .to_string()
}

// Resolve "host" or "user@host" to its tailscale IP so ssh works even without
// MagicDNS in the system resolver. Unknown names pass through untouched.
fn resolve(host: &str) -> Result<String> {
    let (user, name) = match host.split_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, host),
    };
    let v = tailnet()?;
    let peers = v["Peer"].as_object().into_iter().flat_map(|m| m.values());
    for node in std::iter::once(&v["Self"]).chain(peers) {
        if dns_label(node).eq_ignore_ascii_case(name)
            || node["HostName"].as_str().is_some_and(|h| h.eq_ignore_ascii_case(name))
        {
            let ip = node["TailscaleIPs"][0].as_str().unwrap_or(name);
            return Ok(match user {
                Some(u) => format!("{u}@{ip}"),
                None => ip.to_string(),
            });
        }
    }
    Ok(host.to_string())
}

fn ls() -> Result<()> {
    let v = tailnet()?;

    let mut rows: Vec<(String, String, String, String)> = vec![];
    let mut push = |node: &serde_json::Value, me: bool| {
        let name = dns_label(node);
        let ip = node["TailscaleIPs"][0].as_str().unwrap_or("?").to_string();
        let os = node["OS"].as_str().unwrap_or("?").to_string();
        let state = if me {
            "this machine".to_string()
        } else if node["Online"].as_bool().unwrap_or(false) {
            "online".to_string()
        } else {
            "offline".to_string()
        };
        rows.push((name, ip, os, state));
    };

    push(&v["Self"], true);
    if let Some(peers) = v["Peer"].as_object() {
        let mut peers: Vec<_> = peers.values().collect();
        peers.sort_by_key(|p| dns_label(p));
        for p in peers {
            push(p, false);
        }
    }

    let w = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    for (name, ip, os, state) in rows {
        println!("{name:w$}  {ip:15}  {os:7}  {state}");
    }
    Ok(())
}

fn sessions(host: &str) -> Result<()> {
    let target = resolve(host)?;
    let script = format!("{REMOTE_PATH} tmux ls 2>/dev/null || echo 'no tmux sessions'");
    let status = Command::new("ssh").arg(target).arg(script).status()?;
    if !status.success() {
        bail!("ssh {host} failed");
    }
    Ok(())
}

fn ssh(host: &str, session: &str) -> Result<()> {
    let target = resolve(host)?;
    let session = sanitize(session);
    let script = format!("{REMOTE_PATH} exec tmux new-session -A -s {}", quote(&session));
    // Replaces this process with ssh so the terminal is fully interactive.
    let err = Command::new("ssh").arg("-t").arg(target).arg(script).exec();
    bail!("exec ssh failed: {err}");
}

fn claude(args: &[String]) -> Result<()> {
    let mut host = None;
    let mut dir = None;
    let mut session = None;
    let mut claude_args: Vec<String> = vec![];

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-s" | "--session" => session = Some(it.next().context("-s needs a value")?.clone()),
            "--" => {
                claude_args = it.cloned().collect();
                break;
            }
            _ if host.is_none() => host = Some(a.clone()),
            _ if dir.is_none() => dir = Some(a.clone()),
            _ => bail!("unexpected argument: {a}\n\n{USAGE}"),
        }
    }
    let host = host.context("usage: skiff claude <host> <dir> [-s name] [-- <claude args>]")?;
    let dir = dir.context("usage: skiff claude <host> <dir> [-s name] [-- <claude args>]")?;
    let session = sanitize(&session.unwrap_or_else(|| {
        dir.trim_end_matches('/').rsplit('/').next().unwrap_or("claude").to_string()
    }));

    let mut cmd = String::from("claude");
    for a in &claude_args {
        cmd.push(' ');
        cmd.push_str(&quote(a));
    }

    // Create the session detached, then send-keys so claude starts inside the
    // login shell tmux spawns (full PATH). Guarded: never type into a session
    // that already exists.
    let s = quote(&session);
    let msg_running = quote(&format!("skiff: session '{session}' already running on {host}"));
    let msg_started = quote(&format!("skiff: started claude in session '{session}' on {host}"));
    let script = format!(
        "{REMOTE_PATH} \
         if tmux has-session -t {s} 2>/dev/null; then \
           echo {msg_running}; \
         else \
           tmux new-session -d -s {s} -c {} && \
           tmux send-keys -t {s} {} Enter && \
           echo {msg_started}; \
         fi",
        quote_path(&dir),
        quote(&cmd),
    );
    let target = resolve(&host)?;
    let status = Command::new("ssh").arg(&target).arg(script).status()?;
    if !status.success() {
        bail!("ssh {host} failed");
    }
    println!("attach with: skiff ssh {host} {session}");
    Ok(())
}

// tmux session names cannot contain '.' or ':'
fn sanitize(name: &str) -> String {
    name.replace(['.', ':'], "-")
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// Leave a leading ~/ unquoted so the remote shell expands it.
fn quote_path(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("~/{}", quote(rest)),
        None if p == "~" => "~".to_string(),
        None => quote(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("fix the tests"), "'fix the tests'");
        assert_eq!(quote("don't"), r"'don'\''t'");
        assert_eq!(quote_path("~/dev/api"), "~/'dev/api'");
        assert_eq!(quote_path("/abs/path"), "'/abs/path'");
        assert_eq!(sanitize("my.proj:x"), "my-proj-x");
    }
}
