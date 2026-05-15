use anyhow::{Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize, Default)]
struct WtConfig {
    #[serde(default)]
    shared: Vec<String>,
    #[serde(default)]
    post_create: Option<String>,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("init") => init(&args[2..]),
        Some("adopt") => adopt(&args[2..]),
        Some("new") => new(&args[2..]),
        Some("rm") => rm(&args[2..]),
        Some(cmd) => bail!("unknown command: {cmd}"),
        None => bail!("usage: wt <command>"),
    }
}

fn init(args: &[String]) -> Result<()> {
    let [url, parent] = args else {
        bail!("usage: wt init <url> <parent>");
    };
    let parent = Path::new(parent);
    if parent.exists() && std::fs::read_dir(parent)?.next().is_some() {
        bail!("parent directory is not empty: {}", parent.display());
    }
    std::fs::create_dir_all(parent)?;

    let bare = parent.join(".bare");
    run_git(parent, &["init", "--bare", ".bare"])?;
    run_git(&bare, &["remote", "add", "origin", url])?;
    run_git(&bare, &["fetch", "origin"])?;
    run_git(&bare, &["remote", "set-head", "origin", "--auto"])?;

    std::fs::write(parent.join(".git"), "gitdir: ./.bare\n")?;

    let origin_head = capture_git(&bare, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])?;
    let default_branch = origin_head
        .strip_prefix("origin/")
        .ok_or_else(|| anyhow::anyhow!("unexpected origin/HEAD: {origin_head}"))?
        .to_string();

    run_git(
        &bare,
        &[
            "update-ref",
            &format!("refs/heads/{default_branch}"),
            &format!("refs/remotes/origin/{default_branch}"),
        ],
    )?;
    run_git(
        &bare,
        &["symbolic-ref", "HEAD", &format!("refs/heads/{default_branch}")],
    )?;

    let wt_path = parent.join(&default_branch);
    run_git(
        &bare,
        &[
            "worktree",
            "add",
            wt_path.to_str().unwrap(),
            &default_branch,
        ],
    )?;

    Ok(())
}

fn adopt(args: &[String]) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let path = match args {
        [] => std::env::current_dir()?,
        [p] => PathBuf::from(p),
        _ => bail!("usage: wt adopt [path]"),
    };
    let path = path.canonicalize()?;

    let dotgit = path.join(".git");
    if !dotgit.is_dir() {
        bail!(
            "expected a regular git clone with a .git directory at {}",
            path.display()
        );
    }

    let status = capture_git(&path, &["status", "--porcelain"])?;
    if !status.is_empty() {
        bail!(
            "working tree is dirty; commit or stash before adopting:\n{status}"
        );
    }

    let branch = capture_git(&path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == "HEAD" {
        bail!("detached HEAD — adopt requires a named branch");
    }

    if std::fs::metadata(&dotgit)?.dev() != std::fs::metadata(&path)?.dev() {
        bail!(
            ".git and {} are on different filesystems",
            path.display()
        );
    }

    let wt_path = path.join(&branch);
    if wt_path.exists() {
        bail!(
            "{} already exists; cannot create worktree dir there",
            wt_path.display()
        );
    }

    // Bail early if .git/worktrees/ is not writable (e.g. root-owned from a prior
    // privileged worktree). Catching this before any file moves avoids leaving the
    // repo in a half-adopted, unrecoverable state.
    let worktrees_dir = dotgit.join("worktrees");
    if worktrees_dir.exists() {
        let probe = worktrees_dir.join(".wt-probe");
        std::fs::create_dir(&probe)
            .and_then(|_| std::fs::remove_dir(&probe))
            .map_err(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "$(whoami)".into());
                anyhow::anyhow!(
                    "{} is not writable — likely root-owned from a prior privileged worktree.\n\
                     Fix with:\n  sudo chown -R {user}:staff {}",
                    worktrees_dir.display(),
                    worktrees_dir.display()
                )
            })?;
    }

    let wt_top_component = branch.split('/').next().unwrap_or("");

    let candidates = scan_top_level_ignored(&path)?;
    let shared = prompt_for_shared(&candidates)?;

    let entries_to_move: Vec<std::ffi::OsString> = std::fs::read_dir(&path)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| n != ".git")
        .collect();

    if entries_to_move
        .iter()
        .any(|n| n.to_string_lossy() == wt_top_component)
    {
        bail!(
            "a top-level entry named '{wt_top_component}' would collide with the worktree path for branch '{branch}'"
        );
    }

    std::fs::create_dir_all(&wt_path)?;
    for name in &entries_to_move {
        std::fs::rename(path.join(name), wt_path.join(name))?;
    }

    for s in &shared {
        let src = wt_path.join(s);
        let dst = path.join(s);
        if src.exists() {
            if let Some(parent_dir) = dst.parent() {
                std::fs::create_dir_all(parent_dir)?;
            }
            std::fs::rename(&src, &dst)?;
        }
    }

    let bare = path.join(".bare");
    std::fs::rename(&dotgit, &bare)?;
    run_git(&bare, &["config", "core.bare", "true"])?;
    // Best-effort — origin may be unreachable; not fatal.
    let _ = Command::new("git")
        .current_dir(&bare)
        .args(["remote", "set-head", "origin", "--auto"])
        .status();

    let wt_id = branch.replace('/', "-");
    let wt_admin = bare.join("worktrees").join(&wt_id);
    std::fs::create_dir_all(&wt_admin)?;
    std::fs::write(wt_admin.join("HEAD"), format!("ref: refs/heads/{branch}\n"))?;
    std::fs::write(wt_admin.join("commondir"), "../..\n")?;
    let wt_gitfile = wt_path.join(".git");
    std::fs::write(wt_admin.join("gitdir"), format!("{}\n", wt_gitfile.display()))?;
    std::fs::write(
        &wt_gitfile,
        format!("gitdir: {}\n", wt_admin.display()),
    )?;
    // Populate the worktree's index from HEAD so `git status` sees a clean tree.
    run_git(&wt_path, &["read-tree", "HEAD"])?;

    apply_shared_files(&path, &wt_path, &shared)?;

    let config_content = format!(
        "shared = [{}]\n",
        shared
            .iter()
            .map(|s| format!("{s:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    std::fs::write(bare.join("wt-config.toml"), config_content)?;

    std::fs::write(path.join(".git"), "gitdir: ./.bare\n")?;

    println!("Adopted {} → {}", path.display(), wt_path.display());
    Ok(())
}

fn scan_top_level_ignored(path: &Path) -> Result<Vec<String>> {
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            anyhow::anyhow!("non-utf8 filename in {}", path.display())
        })?;
        if name == ".git" {
            continue;
        }
        let check = Command::new("git")
            .current_dir(path)
            .args(["check-ignore", "-q", "--"])
            .arg(&name)
            .status()?;
        if check.success() {
            candidates.push(name);
        }
    }
    candidates.sort();
    Ok(candidates)
}

fn prompt_for_shared(candidates: &[String]) -> Result<Vec<String>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let defaults: Vec<bool> = candidates.iter().map(|c| c.starts_with(".env")).collect();
    let selection = dialoguer::MultiSelect::new()
        .with_prompt("Share across all worktrees? (space to toggle, enter to confirm)")
        .items(candidates)
        .defaults(&defaults)
        .interact()?;
    Ok(selection.into_iter().map(|i| candidates[i].clone()).collect())
}

fn new(args: &[String]) -> Result<()> {
    let (branch, base) = match args {
        [b] => (b.as_str(), None),
        [b, base] => (b.as_str(), Some(base.as_str())),
        _ => bail!("usage: wt new <branch> [base]"),
    };

    // Resolve "." against cwd BEFORE chdir-ing into the bare repo.
    let base_resolved = match base {
        Some(".") => Some(resolve_cwd_head()?),
        Some(other) => Some(other.to_string()),
        None => None,
    };

    let parent = find_parent()?;
    let bare = parent.join(".bare");

    run_git(&bare, &["fetch", "origin"])?;

    if let Some(existing) = existing_worktree_for_branch(&bare, branch)? {
        bail!(
            "branch '{branch}' already has a worktree at {}",
            existing.display()
        );
    }

    let wt_path = parent.join(branch);
    if let Some(dir) = wt_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let wt_path_str = wt_path.to_str().unwrap();

    let has_remote = ref_exists(&bare, &format!("refs/remotes/origin/{branch}"))?;

    if has_remote {
        run_git(
            &bare,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                wt_path_str,
                &format!("origin/{branch}"),
            ],
        )?;
    } else {
        let start = base_resolved.as_deref().unwrap_or("origin/HEAD");
        run_git(
            &bare,
            &[
                "worktree",
                "add",
                "--no-track",
                "-b",
                branch,
                wt_path_str,
                start,
            ],
        )?;
    }

    let config = load_config(&bare)?;
    apply_shared_files(&parent, &wt_path, &config.shared)?;

    if let Some(hook) = &config.post_create {
        let status = Command::new("sh")
            .arg("-c")
            .arg(hook)
            .current_dir(&wt_path)
            .status()?;
        if !status.success() {
            bail!("post_create hook failed: {hook}");
        }
    }

    Ok(())
}

fn load_config(bare: &Path) -> Result<WtConfig> {
    let path = bare.join("wt-config.toml");
    if !path.exists() {
        return Ok(WtConfig::default());
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&content)?)
}

fn apply_shared_files(parent: &Path, wt_path: &Path, shared: &[String]) -> Result<()> {
    use std::os::unix::fs::symlink;

    for item in shared {
        let source_rel = Path::new(item);
        let source_abs = parent.join(source_rel);
        if !source_abs.exists() {
            continue;
        }
        let target_abs = wt_path.join(source_rel);
        if let Some(dir) = target_abs.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let target_dir = target_abs.parent().unwrap();
        let depth = target_dir.strip_prefix(parent)?.components().count();
        let mut link = PathBuf::new();
        for _ in 0..depth {
            link.push("..");
        }
        link.push(source_rel);
        symlink(&link, &target_abs)?;
    }
    Ok(())
}

fn rm(args: &[String]) -> Result<()> {
    let mut force = false;
    let mut positional: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--force" | "-f" => force = true,
            other => positional.push(other),
        }
    }
    let [branch] = positional[..] else {
        bail!("usage: wt rm <branch> [--force]");
    };

    let parent = find_parent()?;
    let bare = parent.join(".bare");
    let wt_path = parent.join(branch);

    if !force && is_unpushed(&bare, branch)? {
        bail!(
            "branch '{branch}' has unpushed commits or no upstream (use --force to override)"
        );
    }

    let mut remove_args = vec!["worktree", "remove"];
    if force {
        remove_args.push("--force");
    }
    let wt_path_str = wt_path.to_str().unwrap();
    remove_args.push(wt_path_str);
    run_git(&bare, &remove_args)?;

    run_git(&bare, &["branch", "-D", branch])?;

    cleanup_empty_parents(&wt_path, &parent)?;

    run_git(&bare, &["worktree", "prune"])?;

    Ok(())
}

fn cleanup_empty_parents(start: &Path, stop_at: &Path) -> Result<()> {
    let mut dir = start.parent();
    while let Some(d) = dir {
        if d == stop_at {
            break;
        }
        match std::fs::remove_dir(d) {
            Ok(()) => {}
            Err(_) => break, // not empty, or doesn't exist
        }
        dir = d.parent();
    }
    Ok(())
}

fn resolve_cwd_head() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !output.status.success() {
        bail!("cannot resolve '.': not inside a git worktree");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn existing_worktree_for_branch(bare: &Path, branch: &str) -> Result<Option<PathBuf>> {
    let output = capture_git(bare, &["worktree", "list", "--porcelain"])?;
    let needle = format!("branch refs/heads/{branch}");
    let mut current_path: Option<String> = None;
    for line in output.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = Some(p.to_string());
        } else if line == needle {
            return Ok(current_path.map(PathBuf::from));
        } else if line.is_empty() {
            current_path = None;
        }
    }
    Ok(None)
}

fn is_unpushed(bare: &Path, branch: &str) -> Result<bool> {
    let upstream_check = Command::new("git")
        .current_dir(bare)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{branch}@{{upstream}}"),
        ])
        .output()?;
    if !upstream_check.status.success() {
        return Ok(true);
    }
    let count = capture_git(
        bare,
        &[
            "rev-list",
            "--count",
            &format!("{branch}@{{upstream}}..{branch}"),
        ],
    )?;
    Ok(count.parse::<u64>().unwrap_or(0) > 0)
}

fn ref_exists(bare: &Path, refname: &str) -> Result<bool> {
    let status = Command::new("git")
        .current_dir(bare)
        .args(["show-ref", "--verify", "--quiet", refname])
        .status()?;
    Ok(status.success())
}

fn find_parent() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .output()?;
    if !output.status.success() {
        bail!("not inside a wt project (no git repo found)");
    }
    let raw = String::from_utf8(output.stdout)?.trim().to_string();
    let git_common = PathBuf::from(&raw);
    let absolute = if git_common.is_absolute() {
        git_common
    } else {
        std::env::current_dir()?.join(git_common)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| anyhow::anyhow!("git common dir has no parent: {}", absolute.display()))?
        .canonicalize()?;
    Ok(parent)
}

fn run_git(dir: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git").current_dir(dir).args(args).status()?;
    if !status.success() {
        bail!("git {args:?} in {dir:?} failed");
    }
    Ok(())
}

fn capture_git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(dir).args(args).output()?;
    if !output.status.success() {
        bail!(
            "git {args:?} in {dir:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}
