// skiff — ferry between your tailscale machines.
//
//   skiff ls [--json]                 list machines on the tailnet
//   skiff sessions <host> [--json]    list tmux sessions on a machine
//   skiff ssh <host> [session]        ssh in, attach-or-create tmux session (default: main)
//   skiff exec <host> -- <cmd...>     run a command on <host>, exit with its exit code
//   skiff logs <host> <session> [-n N]
//                                     print the last N lines of a session's pane
//   skiff kill <host> <session>       kill a tmux session on a machine
//   skiff claude <host> <dir> [-s name] [--json] [-- <claude args>]
//                                     start claude in a detached tmux session on <host>
//   skiff setup <host> [--user <user>] [--nick <nick>]
//                                     interactively persist a nickname + user
//                                     into ~/.ssh/config (flags skip prompts)
//
// Every entry point lands in a named tmux session on the remote machine, so
// work survives disconnects and you can reattach later — via `skiff ssh` or a
// plain `tmux attach` on the machine itself. The `--json` flags and
// exec/logs/kill exist so orchestrating agents can start, watch, and stop
// remote claude sessions without shelling out to raw ssh.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

// Remote non-interactive shells often miss homebrew/cargo paths; tmux and
// claude live there. Prepended to every remote command.
const REMOTE_PATH: &str = r#"export PATH="$PATH:/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.cargo/bin";"#;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ls") => ls(&args[1..]),
        Some("sessions") => sessions(&args[1..]),
        Some("ssh") => ssh(
            args.get(1).context("usage: skiff ssh <host> [session]")?,
            args.get(2).map(String::as_str).unwrap_or("main"),
        ),
        Some("exec") => exec(&args[1..]),
        Some("logs") => logs(&args[1..]),
        Some("kill") => kill(&args[1..]),
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
  skiff ls [--json]                            list machines on the tailnet
  skiff sessions <host> [--json]               list tmux sessions on <host>
  skiff ssh <host> [session]                   attach-or-create tmux session (default: main)
  skiff exec <host> -- <cmd...>                run a command on <host> non-interactively,
                                               exit with the remote command's exit code
  skiff logs <host> <session> [-n N]           print the last N lines of the session's
                                               pane (default 200, no ansi escapes)
  skiff kill <host> <session>                  kill a tmux session on <host>
  skiff claude <host> <dir> [-s name] [--json] [-- <claude args>]
                                               start claude in a detached tmux session
                                               (session name defaults to the dir basename)
  skiff setup <host> [--user <user>] [--nick <nick>]
                                               interactively persist a nickname + user
                                               into ~/.ssh/config (prompts for anything
                                               not passed as a flag)

examples:
  skiff claude work-mac ~/dev/api -- \"fix the failing tests\"
  skiff logs work-mac api -n 50                # what has claude printed lately?
  skiff exec work-mac -- git -C dev/api status
  skiff ssh work-mac api                       # attach to that session later
  skiff kill work-mac api                      # done with it
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
    if ssh_config_has_host(name) {
        return Ok(host.to_string());
    }
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

fn ls(args: &[String]) -> Result<()> {
    let mut json = false;
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            _ => bail!("unexpected argument: {a}\n\n{USAGE}"),
        }
    }

    let v = tailnet()?;
    let nodes = ls_json(&v);
    if json {
        println!("{}", serde_json::to_string(&nodes)?);
        return Ok(());
    }

    let rows = nodes.as_array().cloned().unwrap_or_default();
    let w = rows.iter().map(|n| n["name"].as_str().unwrap_or("").len()).max().unwrap_or(0);
    for n in &rows {
        let name = n["name"].as_str().unwrap_or("?");
        let ip = n["ip"].as_str().unwrap_or("?");
        let os = n["os"].as_str().unwrap_or("?");
        let state = if n["self"].as_bool().unwrap_or(false) {
            "this machine"
        } else if n["online"].as_bool().unwrap_or(false) {
            "online"
        } else {
            "offline"
        };
        println!("{name:w$}  {ip:15}  {os:7}  {state}");
    }
    Ok(())
}

// Shape `tailscale status --json` into what agents need: this machine first,
// peers sorted by name.
fn ls_json(v: &serde_json::Value) -> serde_json::Value {
    let node_json = |node: &serde_json::Value, me: bool| {
        serde_json::json!({
            "name": dns_label(node),
            "ip": node["TailscaleIPs"][0].as_str().unwrap_or("?"),
            "os": node["OS"].as_str().unwrap_or("?"),
            "online": me || node["Online"].as_bool().unwrap_or(false),
            "self": me,
        })
    };
    let mut nodes = vec![node_json(&v["Self"], true)];
    if let Some(peers) = v["Peer"].as_object() {
        let mut peers: Vec<_> = peers.values().collect();
        peers.sort_by_key(|p| dns_label(p));
        for p in peers {
            nodes.push(node_json(p, false));
        }
    }
    serde_json::Value::Array(nodes)
}

// Best-effort push of the local TERM's terminfo to the remote, so a remote
// tmux/ssh doesn't reject an unusual local TERM (e.g. xterm-ghostty) that it
// has no terminfo entry for. Never blocks or fails the caller: missing
// infocmp/tic, a dead ssh, non-zero exits — all swallowed.
fn ensure_remote_terminfo(target: &str) {
    let Ok(term) = std::env::var("TERM") else { return };
    if !term_is_usable(&term) {
        return;
    }
    let Ok(infocmp) = Command::new("infocmp").args(["-x", "--", &term]).output() else { return };
    if !infocmp.status.success() {
        return;
    }
    let Ok(mut child) = Command::new("ssh")
        .arg(target)
        .arg("tic -x - 2>/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&infocmp.stdout);
    }
    let _ = child.wait();
}

// Empty TERM (unset or blank) short-circuits ensure_remote_terminfo.
fn term_is_usable(term: &str) -> bool {
    !term.is_empty()
}

// Tab-separated tmux format strings; parsed back by sessions_json(). The pane
// current command is how an agent tells whether claude is still running.
const TMUX_SESSIONS_FMT: &str = "#{session_name}\t#{session_created}\t#{session_windows}\t#{session_attached}";
const TMUX_PANES_FMT: &str = "#{session_name}\t#{pane_current_command}\t#{pane_dead}";
const PANES_MARKER: &str = "---skiff-panes---";

fn sessions(args: &[String]) -> Result<()> {
    let usage = "usage: skiff sessions <host> [--json]";
    let mut host = None;
    let mut json = false;
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            _ if host.is_none() => host = Some(a.clone()),
            _ => bail!("unexpected argument: {a}\n\n{USAGE}"),
        }
    }
    let host = host.context(usage)?;
    let target = resolve(&host)?;
    ensure_remote_terminfo(&target);

    if !json {
        let script = format!("{REMOTE_PATH} tmux ls 2>/dev/null || echo 'no tmux sessions'");
        let status = Command::new("ssh").arg(target).arg(script).status()?;
        if !status.success() {
            bail!("ssh {host} failed");
        }
        return Ok(());
    }

    // One round-trip: sessions, a marker line, then every pane. Both tmux
    // calls are allowed to fail (no server running == no sessions).
    let script = format!(
        "{REMOTE_PATH} tmux list-sessions -F {} 2>/dev/null; echo {PANES_MARKER}; \
         tmux list-panes -a -F {} 2>/dev/null; true",
        quote(TMUX_SESSIONS_FMT),
        quote(TMUX_PANES_FMT),
    );
    let out = Command::new("ssh").arg(target).arg(script).output()?;
    if !out.status.success() {
        bail!("ssh {host} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (sess, panes) = text.split_once(PANES_MARKER).unwrap_or((text.as_ref(), ""));
    println!("{}", serde_json::to_string(&sessions_json(sess, panes))?);
    Ok(())
}

// Parse the tab-separated tmux output back into one object per session. The
// pane fields come from the session's first pane; sessions with no pane line
// get an empty pane_command.
fn sessions_json(sessions_out: &str, panes_out: &str) -> serde_json::Value {
    let mut pane: std::collections::HashMap<&str, (&str, bool)> = std::collections::HashMap::new();
    for line in panes_out.lines() {
        let mut f = line.split('\t');
        if let (Some(name), Some(cmd), Some(dead)) = (f.next(), f.next(), f.next())
            && !name.is_empty()
        {
            pane.entry(name).or_insert((cmd, dead.trim() == "1"));
        }
    }

    let mut arr = vec![];
    for line in sessions_out.lines() {
        let mut f = line.split('\t');
        let (Some(name), Some(created), Some(windows), Some(attached)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let (pane_command, pane_dead) = pane.get(name).copied().unwrap_or(("", false));
        arr.push(serde_json::json!({
            "name": name,
            "created": created.trim().parse::<u64>().unwrap_or(0),
            "windows": windows.trim().parse::<u64>().unwrap_or(0),
            "attached": attached.trim().parse::<u64>().unwrap_or(0) > 0,
            "pane_command": pane_command,
            "pane_dead": pane_dead,
        }));
    }
    serde_json::Value::Array(arr)
}

fn ssh(host: &str, session: &str) -> Result<()> {
    let target = resolve(host)?;
    ensure_remote_terminfo(&target);
    let session = sanitize(session);
    let script = format!("{REMOTE_PATH} exec tmux new-session -A -s {}", quote(&session));
    // Replaces this process with ssh so the terminal is fully interactive.
    let err = Command::new("ssh").arg("-t").arg(target).arg(script).exec();
    bail!("exec ssh failed: {err}");
}

fn exec(args: &[String]) -> Result<()> {
    let (host, cmd) = parse_exec_args(args)?;
    let target = resolve(&host)?;
    // BatchMode: fail fast instead of hanging on a password prompt — this verb
    // is for agents and scripts, never interactive.
    let status = Command::new("ssh")
        .args(["-o", "BatchMode=yes"])
        .arg(target)
        .arg(exec_script(&cmd))
        .status()?;
    // Mirror the remote command's exit code so callers can branch on it.
    std::process::exit(status.code().unwrap_or(1));
}

fn parse_exec_args(args: &[String]) -> Result<(String, Vec<String>)> {
    let usage = "usage: skiff exec <host> -- <cmd...>";
    let host = args.first().context(usage)?.clone();
    if args.get(1).map(String::as_str) != Some("--") {
        bail!("{usage}");
    }
    let cmd = args[2..].to_vec();
    if cmd.is_empty() {
        bail!("{usage}");
    }
    Ok((host, cmd))
}

fn exec_script(cmd: &[String]) -> String {
    let quoted: Vec<String> = cmd.iter().map(|a| quote(a)).collect();
    format!("{REMOTE_PATH} {}", quoted.join(" "))
}

fn logs(args: &[String]) -> Result<()> {
    let (host, session, n) = parse_logs_args(args)?;
    let target = resolve(&host)?;
    let s = quote(&session);
    // Plain -p (no -e): agents read this, ansi escapes would only get in the way.
    let script = format!(
        "{REMOTE_PATH} tmux has-session -t {s} 2>/dev/null || exit 3; \
         tmux capture-pane -p -t {s} -S -{n}"
    );
    let status = Command::new("ssh").arg(target).arg(script).status()?;
    match status.code() {
        Some(0) => Ok(()),
        Some(3) => bail!("skiff: no session '{session}' on {host} — see `skiff sessions {host}`"),
        _ => bail!("ssh {host} failed"),
    }
}

fn parse_logs_args(args: &[String]) -> Result<(String, String, u32)> {
    let usage = "usage: skiff logs <host> <session> [-n N]";
    let mut host = None;
    let mut session = None;
    let mut n: u32 = 200;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-n" | "--lines" => {
                let v = it.next().context("-n needs a value")?;
                n = v.parse().with_context(|| format!("-n needs a number, got '{v}'"))?;
            }
            _ if host.is_none() => host = Some(a.clone()),
            _ if session.is_none() => session = Some(sanitize(a)),
            _ => bail!("unexpected argument: {a}\n\n{USAGE}"),
        }
    }
    Ok((host.context(usage)?, session.context(usage)?, n))
}

fn kill(args: &[String]) -> Result<()> {
    let usage = "usage: skiff kill <host> <session>";
    let host = args.first().context(usage)?;
    let session = sanitize(args.get(1).context(usage)?);
    if let Some(extra) = args.get(2) {
        bail!("unexpected argument: {extra}\n\n{USAGE}");
    }
    let target = resolve(host)?;
    let s = quote(&session);
    let script =
        format!("{REMOTE_PATH} tmux has-session -t {s} 2>/dev/null || exit 3; tmux kill-session -t {s}");
    let status = Command::new("ssh").arg(target).arg(script).status()?;
    match status.code() {
        Some(0) => {
            println!("skiff: killed session '{session}' on {host}");
            Ok(())
        }
        Some(3) => bail!("skiff: no session '{session}' on {host} — see `skiff sessions {host}`"),
        _ => bail!("ssh {host} failed"),
    }
}

struct ClaudeArgs {
    host: String,
    dir: String,
    session: String,
    json: bool,
    claude_args: Vec<String>,
}

fn parse_claude_args(args: &[String]) -> Result<ClaudeArgs> {
    let usage = "usage: skiff claude <host> <dir> [-s name] [--json] [-- <claude args>]";
    let mut host = None;
    let mut dir = None;
    let mut session = None;
    let mut json = false;
    let mut claude_args: Vec<String> = vec![];

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-s" | "--session" => session = Some(it.next().context("-s needs a value")?.clone()),
            "--json" => json = true,
            "--" => {
                claude_args = it.cloned().collect();
                break;
            }
            _ if host.is_none() => host = Some(a.clone()),
            _ if dir.is_none() => dir = Some(a.clone()),
            _ => bail!("unexpected argument: {a}\n\n{USAGE}"),
        }
    }
    let host = host.context(usage)?;
    let dir = dir.context(usage)?;
    let session = sanitize(&session.unwrap_or_else(|| {
        dir.trim_end_matches('/').rsplit('/').next().unwrap_or("claude").to_string()
    }));
    Ok(ClaudeArgs { host, dir, session, json, claude_args })
}

fn claude_json(host: &str, session: &str, status: &str) -> serde_json::Value {
    serde_json::json!({
        "host": host,
        "session": session,
        "status": status,
        "attach_cmd": format!("skiff ssh {host} {session}"),
    })
}

// Bail loudly if tmux or claude is missing on the remote — without this, a
// bare host would print "started" while send-keys typed into a dead shell.
fn check_remote_prereqs(host: &str, target: &str) -> Result<()> {
    let script = format!(
        "{REMOTE_PATH} missing=''; \
         command -v tmux >/dev/null 2>&1 || missing=\"$missing tmux\"; \
         command -v claude >/dev/null 2>&1 || missing=\"$missing claude\"; \
         printf '%s' \"$missing\""
    );
    let out = Command::new("ssh").arg(target).arg(script).output()?;
    if !out.status.success() {
        bail!("ssh {host} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let missing = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !missing.is_empty() {
        bail!(
            "skiff: {host} is missing {} — run install.sh on that machine first",
            missing.split_whitespace().collect::<Vec<_>>().join(" and ")
        );
    }
    Ok(())
}

fn claude(args: &[String]) -> Result<()> {
    let a = parse_claude_args(args)?;

    let mut cmd = String::from("claude");
    for arg in &a.claude_args {
        cmd.push(' ');
        cmd.push_str(&quote(arg));
    }

    let target = resolve(&a.host)?;
    ensure_remote_terminfo(&target);
    check_remote_prereqs(&a.host, &target)?;

    // Create the session detached, then send-keys so claude starts inside the
    // login shell tmux spawns (full PATH). Guarded: never type into a session
    // that already exists. The remote echoes a status token we parse locally.
    let s = quote(&a.session);
    let script = format!(
        "{REMOTE_PATH} \
         if tmux has-session -t {s} 2>/dev/null; then \
           echo already-running; \
         else \
           tmux new-session -d -s {s} -c {} && \
           tmux send-keys -t {s} {} Enter && \
           echo started; \
         fi",
        quote_path(&a.dir),
        quote(&cmd),
    );
    let out = Command::new("ssh").arg(&target).arg(script).output()?;
    if !out.status.success() {
        bail!("ssh {} failed: {}", a.host, String::from_utf8_lossy(&out.stderr).trim());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let status = match stdout.trim() {
        "started" => "started",
        "already-running" => "already-running",
        other => bail!(
            "skiff: starting claude on {} failed: {}{}",
            a.host,
            other,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    };

    if a.json {
        println!("{}", serde_json::to_string(&claude_json(&a.host, &a.session, status))?);
    } else {
        match status {
            "started" => println!("skiff: started claude in session '{}' on {}", a.session, a.host),
            _ => println!("skiff: session '{}' already running on {}", a.session, a.host),
        }
        println!("attach with: skiff ssh {} {}", a.host, a.session);
    }
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

// Exact-token match on `Host` lines in ~/.ssh/config; not a full implementation
// of ssh's wildcard Host matching.
fn ssh_config_has_host(name: &str) -> bool {
    let path = match std::env::var("HOME") {
        Ok(home) => home + "/.ssh/config",
        Err(_) => return false,
    };
    match fs::read_to_string(&path) {
        Ok(contents) => config_has_host(&contents, name),
        Err(_) => false,
    }
}

fn config_has_host(contents: &str, name: &str) -> bool {
    contents
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Host "))
        .flat_map(|rest| rest.split_whitespace())
        .any(|token| token == name)
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
    fn exec_script_quotes_every_arg() {
        let cmd: Vec<String> =
            ["git", "commit", "-m", "don't"].iter().map(|s| s.to_string()).collect();
        assert_eq!(exec_script(&cmd), format!(r"{REMOTE_PATH} 'git' 'commit' '-m' 'don'\''t'"));
    }

    #[test]
    fn parse_exec_args_requires_dashdash_and_cmd() {
        let ok = |s: &[&str]| parse_exec_args(&s.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        let (host, cmd) = ok(&["wm", "--", "uptime"]).unwrap();
        assert_eq!(host, "wm");
        assert_eq!(cmd, vec!["uptime"]);
        assert!(ok(&[]).is_err());
        assert!(ok(&["wm"]).is_err());
        assert!(ok(&["wm", "uptime"]).is_err(), "cmd without -- is rejected");
        assert!(ok(&["wm", "--"]).is_err(), "empty cmd is rejected");
    }

    #[test]
    fn parse_logs_args_defaults_and_overrides() {
        let parse = |s: &[&str]| parse_logs_args(&s.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(parse(&["wm", "api"]).unwrap(), ("wm".into(), "api".into(), 200));
        assert_eq!(parse(&["wm", "api", "-n", "50"]).unwrap(), ("wm".into(), "api".into(), 50));
        assert_eq!(parse(&["-n", "50", "wm", "my.proj"]).unwrap(), ("wm".into(), "my-proj".into(), 50));
        assert!(parse(&["wm"]).is_err());
        assert!(parse(&["wm", "api", "-n", "lots"]).is_err());
        assert!(parse(&["wm", "api", "extra"]).is_err());
    }

    #[test]
    fn parse_claude_args_defaults_session_to_dir_basename() {
        let args: Vec<String> = ["wm", "~/dev/api/"].iter().map(|s| s.to_string()).collect();
        let a = parse_claude_args(&args).unwrap();
        assert_eq!(a.host, "wm");
        assert_eq!(a.dir, "~/dev/api/");
        assert_eq!(a.session, "api");
        assert!(!a.json);
        assert!(a.claude_args.is_empty());
    }

    #[test]
    fn parse_claude_args_flags_and_passthrough() {
        let args: Vec<String> = ["wm", "~/dev/api", "--json", "-s", "my.sesh", "--", "fix tests", "--json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let a = parse_claude_args(&args).unwrap();
        assert!(a.json);
        assert_eq!(a.session, "my-sesh");
        // Everything after -- belongs to claude, even flag-looking args.
        assert_eq!(a.claude_args, vec!["fix tests", "--json"]);
    }

    #[test]
    fn claude_json_shape() {
        let v = claude_json("wm", "api", "started");
        assert_eq!(
            v,
            serde_json::json!({
                "host": "wm",
                "session": "api",
                "status": "started",
                "attach_cmd": "skiff ssh wm api",
            })
        );
    }

    #[test]
    fn ls_json_self_first_then_sorted_peers() {
        let status = serde_json::json!({
            "Self": {
                "DNSName": "mymac.tail.ts.net.",
                "HostName": "mymac",
                "TailscaleIPs": ["100.1.1.1"],
                "OS": "macOS",
                "Online": true,
            },
            "Peer": {
                "key-z": {
                    "DNSName": "zeta.tail.ts.net.",
                    "HostName": "zeta",
                    "TailscaleIPs": ["100.3.3.3"],
                    "OS": "linux",
                    "Online": false,
                },
                "key-a": {
                    "DNSName": "alpha.tail.ts.net.",
                    "HostName": "alpha",
                    "TailscaleIPs": ["100.2.2.2"],
                    "OS": "linux",
                    "Online": true,
                },
            },
        });
        assert_eq!(
            ls_json(&status),
            serde_json::json!([
                {"name": "mymac", "ip": "100.1.1.1", "os": "macOS", "online": true, "self": true},
                {"name": "alpha", "ip": "100.2.2.2", "os": "linux", "online": true, "self": false},
                {"name": "zeta", "ip": "100.3.3.3", "os": "linux", "online": false, "self": false},
            ])
        );
    }

    #[test]
    fn sessions_json_joins_first_pane_per_session() {
        let sessions = "api\t1719000000\t2\t1\nmain\t1719000001\t1\t0\n";
        let panes = "api\tclaude\t0\napi\tzsh\t0\nmain\tzsh\t0\n";
        assert_eq!(
            sessions_json(sessions, panes),
            serde_json::json!([
                {"name": "api", "created": 1719000000u64, "windows": 2, "attached": true,
                 "pane_command": "claude", "pane_dead": false},
                {"name": "main", "created": 1719000001u64, "windows": 1, "attached": false,
                 "pane_command": "zsh", "pane_dead": false},
            ])
        );
    }

    #[test]
    fn sessions_json_handles_no_sessions_and_dead_panes() {
        assert_eq!(sessions_json("", ""), serde_json::json!([]));
        assert_eq!(sessions_json("\n", "\n"), serde_json::json!([]));
        let v = sessions_json("api\t1\t1\t0\n", "api\tclaude\t1\n");
        assert_eq!(v[0]["pane_dead"], serde_json::json!(true));
        // A session with no pane line still gets shaped, with empty pane fields.
        let v = sessions_json("bare\t1\t1\t0\n", "");
        assert_eq!(v[0]["pane_command"], serde_json::json!(""));
        assert_eq!(v[0]["pane_dead"], serde_json::json!(false));
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

    #[test]
    fn config_has_host_matches_exact_token() {
        let cfg = "Host other\n    HostName 10.0.0.1\nHost wm\n    HostName 100.1.2.3\n";
        assert!(config_has_host(cfg, "wm"));
        assert!(!config_has_host(cfg, "unrelated"));
    }

    #[test]
    fn config_has_host_matches_any_token_on_multi_host_line() {
        let cfg = "Host wm work-mac\n    HostName 100.1.2.3\n";
        assert!(config_has_host(cfg, "wm"));
        assert!(config_has_host(cfg, "work-mac"));
        assert!(!config_has_host(cfg, "work"));
    }

    #[test]
    fn config_has_host_empty_is_false() {
        assert!(!config_has_host("", "wm"));
    }

    #[test]
    fn term_is_usable_rejects_empty() {
        assert!(!term_is_usable(""));
        assert!(term_is_usable("xterm-ghostty"));
    }
}
