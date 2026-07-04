// skiff — ferry between your tailscale machines.
//
//   skiff ls                          list machines on the tailnet
//   skiff sessions <host>             list tmux sessions on a machine
//   skiff ssh <host> [session]        ssh in, attach-or-create tmux session (default: main)
//   skiff claude <host> <dir> [-s name] [-- <claude args>]
//                                     start claude in a detached tmux session on <host>
//   skiff setup <host> [--user <user>] [--nick <nick>]
//                                     interactively persist a nickname + user
//                                     into ~/.ssh/config (flags skip prompts)
//
// Every entry point lands in a named tmux session on the remote machine, so
// work survives disconnects and you can reattach later — via `skiff ssh` or a
// plain `tmux attach` on the machine itself.

use std::fs;
use std::os::unix::fs::PermissionsExt;
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
        Some("setup") => setup(&args[1..]),
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
  skiff setup <host> [--user <user>] [--nick <nick>]
                                               interactively persist a nickname + user
                                               into ~/.ssh/config (prompts for anything
                                               not passed as a flag)

examples:
  skiff claude work-mac ~/dev/api -- \"fix the failing tests\"
  skiff ssh work-mac api                       # attach to that session later
  skiff setup work-mac                         # prompts for user + nickname
  skiff setup work-mac --user cael --nick wm   # non-interactive
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

fn setup(args: &[String]) -> Result<()> {
    let mut host = None;
    let mut user = None;
    let mut nick = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--user" => user = Some(it.next().context("--user needs a value")?.clone()),
            "--nick" => nick = Some(it.next().context("--nick needs a value")?.clone()),
            _ if host.is_none() => host = Some(a.clone()),
            _ => bail!("unexpected argument: {a}\n\n{USAGE}"),
        }
    }
    let host = host.context("usage: skiff setup <host> [--user <user>] [--nick <nick>]")?;
    let default_nick = host.split('.').next().unwrap_or(&host).to_string();

    let ip = resolve(&host)?;

    // Interactive unless both overrides were passed on the command line.
    let (user, nick, interactive) = match (user, nick) {
        (Some(u), Some(n)) => (u, n, false),
        (user, nick) => {
            println!("skiff: {host} -> {ip}");
            let user = match user {
                Some(u) => u,
                None => prompt_required("username")?,
            };
            let nick = match nick {
                Some(n) => n,
                None => prompt_with_default("nickname", &default_nick)?,
            };
            (user, nick, true)
        }
    };

    let block = ssh_config_block(&nick, &ip, &user);
    if interactive {
        print!("{block}");
        if !confirm("write this to ~/.ssh/config?")? {
            println!("skiff: aborted, nothing written");
            return Ok(());
        }
    }

    let path = std::env::var("HOME").context("HOME not set")? + "/.ssh/config";
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let updated = upsert_block(&existing, &nick, &block);
    fs::write(&path, updated).with_context(|| format!("writing {path}"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;

    println!("skiff: wrote Host {nick} ({user}@{ip}) to {path}");
    println!("connect with: ssh {nick}");
    Ok(())
}

fn read_line(prompt: &str) -> Result<String> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn prompt_required(label: &str) -> Result<String> {
    loop {
        let line = read_line(&format!("{label}: "))?;
        if !line.is_empty() {
            return Ok(line);
        }
        eprintln!("{label} cannot be empty");
    }
}

fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    let line = read_line(&format!("{label} [{default}]: "))?;
    Ok(if line.is_empty() { default.to_string() } else { line })
}

// Default "N" on empty input, consistent with the (y/N) convention shown to the user.
fn confirm(label: &str) -> Result<bool> {
    let line = read_line(&format!("{label} (y/N): "))?;
    Ok(matches!(line.as_str(), "y" | "Y" | "yes"))
}

fn ssh_config_block(nick: &str, ip: &str, user: &str) -> String {
    format!("# >>> skiff {nick}\nHost {nick}\n    HostName {ip}\n    User {user}\n# <<< skiff {nick}\n")
}

// Replaces an existing "# >>> skiff <nick>" ... "# <<< skiff <nick>" block in
// place, or appends the new block if no such block exists yet.
fn upsert_block(existing: &str, nick: &str, block: &str) -> String {
    let start_marker = format!("# >>> skiff {nick}");
    let end_marker = format!("# <<< skiff {nick}");
    if let Some(start) = existing.find(&start_marker)
        && let Some(end_rel) = existing[start..].find(&end_marker)
    {
        let mut end = start + end_rel + end_marker.len();
        if existing[end..].starts_with('\n') {
            end += 1;
        }
        return format!("{}{}{}", &existing[..start], block, &existing[end..]);
    }
    let mut result = existing.to_string();
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result.push_str(block);
    result
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

    #[test]
    fn upsert_block_appends_to_fresh_config() {
        let block = ssh_config_block("wm", "100.1.2.3", "cael");
        let out = upsert_block("", "wm", &block);
        assert_eq!(out, block);
    }

    #[test]
    fn upsert_block_appends_after_unrelated_host() {
        let existing = "Host other\n    HostName 10.0.0.1\n    User bob\n";
        let block = ssh_config_block("wm", "100.1.2.3", "cael");
        let out = upsert_block(existing, "wm", &block);
        assert_eq!(out, format!("{existing}{block}"));
    }

    #[test]
    fn upsert_block_replaces_in_place_on_rerun() {
        let block_v1 = ssh_config_block("wm", "100.1.2.3", "cael");
        let existing = format!("Host other\n    HostName 10.0.0.1\n# comment\n{block_v1}Host another\n    HostName 10.0.0.2\n");
        let block_v2 = ssh_config_block("wm", "100.9.9.9", "cael2");
        let out = upsert_block(&existing, "wm", &block_v2);
        assert_eq!(
            out,
            format!("Host other\n    HostName 10.0.0.1\n# comment\n{block_v2}Host another\n    HostName 10.0.0.2\n")
        );
        assert_eq!(out.matches("Host wm").count(), 1);
    }
}
