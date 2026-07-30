//! Transcript reading: one `.jsonl` file in, deduplicated Requests out.
//!
//! Lenient by contract: a line we cannot understand is counted in `skipped`,
//! never fatal.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The six Categories, in report order.
pub const CATEGORIES: [&str; 6] = ["Prompt", "Tool call", "MCP", "Skill", "Agent", "Overhead"];

/// Record types Claude Code writes that carry no billable usage. Anything
/// outside this list (and outside `assistant`/`user`) counts as skipped.
const NON_BILLABLE: [&str; 16] = [
    "ai-title",
    "attachment",
    "custom-title",
    "file-history-delta",
    "file-history-snapshot",
    "frame-link",
    "last-prompt",
    "mode",
    "permission-mode",
    "pr-link",
    "queue-operation",
    "result",
    "started",
    "summary",
    "system",
    "x-claude-code-hook",
];

/// One API request, the atomic unit of accounting.
pub struct Request {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub fresh: i64,
    pub cache_read: i64,
    pub model: String,
    pub category: &'static str,
    /// For MCP requests, the `<server>` segment of the first `mcp__*` result
    /// that triggered them.
    pub mcp_server: Option<String>,
    /// `attributionSkill` — always None on subagent records.
    pub skill: Option<String>,
}

#[derive(Default)]
pub struct Parsed {
    /// Requests in file line order, deduplicated within the file.
    pub requests: Vec<Request>,
    /// `Agent` tool_use id -> `message.id` of the request that spawned it.
    pub spawns: HashMap<String, String>,
    /// Readable project path, taken from the records' `cwd`.
    pub project: String,
    pub first_prompt: String,
    pub skipped: usize,
}

pub fn parse_file(path: &Path, is_agent_file: bool) -> Parsed {
    let mut p = Parsed::default();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return p,
    };
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    // Best (highest-priority) trigger seen since the last counted request.
    let mut trigger = "Overhead";
    let mut trigger_server: Option<String> = None;
    // Fallback label for sessions that never got a prompt of their own.
    let mut first_command = String::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                p.skipped += 1;
                continue;
            }
        };
        if p.project.is_empty() {
            if let Some(cwd) = v["cwd"].as_str() {
                p.project = project_from_cwd(cwd);
            }
        }
        match v["type"].as_str() {
            Some("assistant") => {
                let counted = read_assistant(
                    &v,
                    is_agent_file,
                    trigger,
                    &trigger_server,
                    &mut seen,
                    &mut tool_names,
                    &mut p,
                );
                if counted {
                    trigger = "Overhead";
                    trigger_server = None;
                }
            }
            Some("user") => {
                let (candidate, server) = user_trigger(&v, &tool_names);
                // Strictly less, so the *first* server to win stays the winner.
                if rank(candidate) < rank(trigger) {
                    trigger = candidate;
                    trigger_server = server;
                }
                if p.first_prompt.is_empty() && v["isMeta"] != Value::Bool(true) {
                    if let Some(body) = plain_text(&v["message"]["content"]) {
                        // Sessions almost always open with a slash command
                        // (`/clear`, `/model`); it names nothing, so keep
                        // looking for what the person actually asked for.
                        let label = truncate(&strip_tags(&body), 80);
                        if body.contains("<command-name>") {
                            if first_command.is_empty() {
                                first_command = label;
                            }
                        } else {
                            p.first_prompt = label;
                        }
                    }
                }
            }
            Some(t) if NON_BILLABLE.contains(&t) => {}
            _ => p.skipped += 1,
        }
    }
    if p.first_prompt.is_empty() {
        p.first_prompt = first_command;
    }
    p
}

/// Several consecutive user records can precede one request — a tool_result
/// followed by the skill body Claude Code injects as an `isMeta` message, say.
/// All of them added context, so the request takes the strongest one.
fn rank(category: &str) -> u8 {
    match category {
        "MCP" => 0,
        "Skill" => 1,
        "Tool call" => 2,
        "Prompt" => 3,
        _ => 4,
    }
}

/// Returns true when a new request was counted (the trigger has been consumed).
fn read_assistant(
    v: &Value,
    is_agent_file: bool,
    trigger: &'static str,
    trigger_server: &Option<String>,
    seen: &mut HashSet<String>,
    tool_names: &mut HashMap<String, String>,
    p: &mut Parsed,
) -> bool {
    let m = &v["message"];
    let id = match m["id"].as_str() {
        Some(s) => s,
        None => return false,
    };
    // Tool names live only on tool_use blocks; harvest every duplicate record so
    // the later id -> name join and the Agent spawn link cannot miss one.
    if let Some(blocks) = m["content"].as_array() {
        for b in blocks {
            if b["type"] != "tool_use" {
                continue;
            }
            if let (Some(tid), Some(name)) = (b["id"].as_str(), b["name"].as_str()) {
                tool_names.insert(tid.to_string(), name.to_string());
                if name == "Agent" || name == "Task" {
                    p.spawns.insert(tid.to_string(), id.to_string());
                }
            }
        }
    }
    let model = m["model"].as_str().unwrap_or("");
    let usage = &m["usage"];
    if model.is_empty() || model == "<synthetic>" || !usage.is_object() {
        return false;
    }
    let ts = match v["timestamp"].as_str().and_then(parse_ts) {
        Some(t) => t,
        None => return false,
    };
    if !seen.insert(id.to_string()) {
        return false; // one API request is written once per content block
    }
    let n = |k: &str| usage[k].as_i64().unwrap_or(0);
    p.requests.push(Request {
        id: id.to_string(),
        ts,
        fresh: n("input_tokens") + n("cache_creation_input_tokens") + n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        model: model.to_string(),
        category: if is_agent_file { "Agent" } else { trigger },
        mcp_server: if is_agent_file { None } else { trigger_server.clone() },
        skill: if is_agent_file {
            None
        } else {
            v["attributionSkill"].as_str().map(str::to_string)
        },
    });
    true
}

/// What one user record added to context, plus the MCP server behind it.
/// Priority among mixed parallel tool results: MCP > Skill > Tool call.
fn user_trigger(
    v: &Value,
    tool_names: &HashMap<String, String>,
) -> (&'static str, Option<String>) {
    let content = &v["message"]["content"];
    let results: Vec<&Value> = match content.as_array() {
        Some(blocks) => blocks.iter().filter(|b| b["type"] == "tool_result").collect(),
        None => Vec::new(),
    };
    if !results.is_empty() {
        let names: Vec<&str> = results
            .iter()
            .filter_map(|b| b["tool_use_id"].as_str())
            .filter_map(|id| tool_names.get(id).map(String::as_str))
            .collect();
        // Results keep their block order, so the first mcp__ hit is the one to
        // attribute a mixed-server request to.
        if let Some(name) = names.iter().find(|n| n.starts_with("mcp__")) {
            return ("MCP", Some(mcp_server(name)));
        }
        if names.iter().any(|n| *n == "Skill") {
            return ("Skill", None);
        }
        return ("Tool call", None);
    }
    let category = match plain_text(content) {
        Some(t) if t.contains("<command-name>") || t.contains("<command-message>") => "Skill",
        Some(_) if v["isMeta"] != Value::Bool(true) => "Prompt",
        _ => "Overhead",
    };
    (category, None)
}

/// `mcp__playwright__browser_click` -> `playwright`.
fn mcp_server(tool: &str) -> String {
    let rest = tool.trim_start_matches("mcp__");
    rest.split("__").next().unwrap_or(rest).to_string()
}

/// Non-empty user-authored text, from either a bare string or `text` blocks.
/// None for tool_result content.
fn plain_text(content: &Value) -> Option<String> {
    let t = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    if t.trim().is_empty() {
        None
    } else {
        Some(t)
    }
}

/// Drop `<...>` spans (command tags, system reminders) and collapse whitespace.
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

/// `/Users/me/dev/nootka-sop` or `C:\Users\me\dev\nootka-sop` -> `dev/nootka-sop`.
/// The record's `cwd` is exact, unlike the flattened project directory name
/// (dashes are ambiguous).
pub fn project_from_cwd(cwd: &str) -> String {
    let norm = cwd.replace('\\', "/");
    let no_drive = if norm.len() >= 2 && norm.as_bytes()[1] == b':' {
        &norm[2..]
    } else {
        norm.as_str()
    };
    let p = no_drive.trim_start_matches('/');
    let mut segs = p.split('/');
    match (segs.next(), segs.next()) {
        (Some("Users"), Some(_)) | (Some("home"), Some(_)) => segs.collect::<Vec<_>>().join("/"),
        _ => p.to_string(),
    }
}

/// Fallback when no record carried a `cwd`: decode the flattened directory name.
/// Lossy — dashes in path segments are indistinguishable from separators.
pub fn project_from_dir(dir: &str) -> String {
    project_from_cwd(&dir.replace('-', "/"))
}
