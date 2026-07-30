//! claude-analyzer: a Reader over the transcripts Claude Code already writes.
//!
//! `analyze` walks `~/.claude/projects`, deduplicates requests by `message.id`,
//! keeps the trailing Week, and returns the report payload as JSON.

pub mod aggregate;
pub mod parse;

use aggregate::Session;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// One transcript file and the session it belongs to.
struct Found {
    path: PathBuf,
    session: String,
    /// Subagent id when this is a `subagents/agent-*.jsonl` file.
    agent: Option<String>,
    /// Flattened project directory name, used only if no record carried a `cwd`.
    project_dir: String,
}

pub fn analyze(root: &Path, now: DateTime<Utc>) -> Value {
    let week_start = aggregate::week_start(now);
    let prev_start = aggregate::prev_week_start(now);
    let mut files = scan(root);
    // A file untouched since before the Previous Week cannot hold in-window
    // records; a day of slack covers clock skew and copied timestamps.
    let cutoff = prev_start - Duration::days(1);
    files.retain(|f| mtime(&f.path).map_or(true, |t| t >= cutoff));
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut sessions: BTreeMap<String, Session> = BTreeMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut skipped = 0usize;

    for f in &files {
        let p = parse::parse_file(&f.path, f.agent.is_some());
        skipped += p.skipped;
        let s = sessions.entry(f.session.clone()).or_insert_with(Session::default);
        if s.project.is_empty() {
            s.project = if p.project.is_empty() {
                parse::project_from_dir(&f.project_dir)
            } else {
                p.project
            };
        }
        // Requests already seen came from a resumed or compacted copy of the
        // same conversation: one API request, one count.
        let fresh: Vec<parse::Request> = p
            .requests
            .into_iter()
            .filter(|r| seen.insert(r.id.clone()) && r.ts >= prev_start && r.ts <= now)
            .collect();

        match &f.agent {
            None => {
                if s.first_prompt.is_empty() {
                    s.first_prompt = p.first_prompt;
                }
                s.spawns.extend(p.spawns);
                s.main.extend(fresh);
            }
            Some(id) => {
                let a = s.agents.entry(id.clone()).or_default();
                a.tool_use_id = agent_tool_use_id(&f.path);
                if let Some(r) = fresh.first() {
                    a.model = r.model.clone();
                }
                a.requests.extend(fresh);
            }
        }
    }
    aggregate::report(sessions, skipped, now, week_start, prev_start)
}

/// The `toolUseId` in the sidecar `agent-<id>.meta.json` — the authoritative
/// link from a subagent transcript back to the `Agent` tool_use that spawned it.
fn agent_tool_use_id(jsonl: &Path) -> Option<String> {
    let meta = jsonl.with_extension("meta.json");
    let text = std::fs::read_to_string(meta).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v["toolUseId"].as_str().map(str::to_string)
}

fn scan(root: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Some(f) = classify(&path) {
                    out.push(f);
                }
            }
        }
    }
    out
}

/// `<project>/<session>.jsonl` is a main transcript;
/// `<project>/<session>/subagents/agent-<id>.jsonl` belongs to that session.
fn classify(path: &Path) -> Option<Found> {
    let stem = path.file_stem()?.to_str()?.to_string();
    let parent = path.parent()?;
    let parent_name = parent.file_name()?.to_str()?.to_string();
    if parent_name == "subagents" {
        let session_dir = parent.parent()?;
        Some(Found {
            path: path.to_path_buf(),
            session: session_dir.file_name()?.to_str()?.to_string(),
            agent: Some(stem.trim_start_matches("agent-").to_string()),
            project_dir: session_dir.parent()?.file_name()?.to_str()?.to_string(),
        })
    } else {
        Some(Found {
            path: path.to_path_buf(),
            session: stem,
            agent: None,
            project_dir: parent_name,
        })
    }
}

fn mtime(path: &Path) -> Option<DateTime<Utc>> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().map(DateTime::<Utc>::from)
}

/// `~/.claude/projects`, the only data source.
pub fn default_root() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    Path::new(&home).join(".claude").join("projects")
}
