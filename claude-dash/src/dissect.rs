//! Context dissection: what is Claude Code actually *sending* in a session, by
//! source — so the setup (CLAUDE.md files, hooks, skills, tools) can be pruned
//! for token efficiency.
//!
//! Two kinds of numbers, labeled in the report:
//! - **exact** — straight from API `usage` (the first request's context size,
//!   assistant output, subagent totals);
//! - **`~` estimates** — `chars / 4` over transcript-visible text.
//!
//! The system prompt, tool definitions, CLAUDE.md contents, and memory are NOT
//! persisted in transcripts. Their combined size is inferred as the first
//! request's context minus everything transcript-visible on turn 1, then
//! sub-attributed by re-reading the CLAUDE.md/MEMORY.md files from disk.
//!
//! Skipped (v1): `<system-reminder>` blocks embedded inside tool_result content
//! (pre-2.1.2xx transcripts — slightly misattributed to the tool there),
//! recursive subagent dissection, a TUI view.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::transcripts::{self, human_prompt_text, usage_of, RawLine};
use crate::tui::humanize_tokens;

/// `chars / 4` — the one place the token-per-char heuristic lives.
fn est_tokens(chars: usize) -> u64 {
    (chars / 4) as u64
}

#[derive(Debug, Default, Serialize)]
pub struct Dissection {
    pub session_id: String,
    pub project: String,
    /// Exact: first non-synthetic request's `input + cache_write + cache_read`.
    pub first_request_tokens: u64,
    /// `first_request_tokens` minus turn-1 transcript-visible content (est).
    pub hidden_base_tokens: u64,
    /// CLAUDE.md / MEMORY.md files re-read from disk: `(label, est tokens)`.
    pub disk_files: Vec<(String, u64)>,
    /// Turn-1 injected attachments (hooks, skill/agent listings): `(label, est)`.
    pub session_start: Vec<(String, u64)>,
    /// Tool results: `(tool name, calls, est tokens)`, largest first.
    pub tools: Vec<(String, u32, u64)>,
    /// Mid-session attachments (diagnostics, task reminders…): `(label, est)`.
    pub mid_injections: Vec<(String, u64)>,
    /// Exact: sum of `usage.output_tokens` (thinking is billed here too).
    pub assistant_output_tokens: u64,
    pub typed_tokens: u64,
    pub command_wrapper_tokens: u64,
    pub subagent_count: u32,
    /// Exact: summed `usage` totals across `subagents/*.jsonl`.
    pub subagent_tokens: u64,
    pub sessions_in_window: usize,
}

/// Everything a single streaming pass over the transcript yields.
#[derive(Default)]
struct Fold {
    first_request_tokens: u64,
    seen_first_assistant: bool,
    cwd: Option<String>,
    /// Attachment label → chars, split at the first assistant record.
    start_attachments: BTreeMap<String, usize>,
    mid_attachments: BTreeMap<String, usize>,
    /// Tool name → (calls, result chars).
    tools: BTreeMap<String, (u32, usize)>,
    /// `tool_use.id → name`, built as we go (uses precede results in file order).
    tool_names: BTreeMap<String, String>,
    assistant_output_tokens: u64,
    typed_chars: usize,
    first_typed_chars: usize,
    wrapper_chars: usize,
}

fn fold_file(path: &Path) -> Result<Fold> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut f = Fold::default();
    for line in std::io::BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        let Ok(raw) = serde_json::from_str::<RawLine>(&line) else {
            continue;
        };
        if f.cwd.is_none() {
            f.cwd = raw.cwd.clone();
        }
        match raw.kind.as_deref() {
            Some("attachment") => {
                if let Some(att) = &raw.attachment {
                    let bucket = if f.seen_first_assistant {
                        &mut f.mid_attachments
                    } else {
                        &mut f.start_attachments
                    };
                    let (label, chars) = attachment_label_and_size(att);
                    *bucket.entry(label).or_default() += chars;
                }
            }
            Some("assistant") => {
                if let Some(u) = usage_of(&raw) {
                    if !f.seen_first_assistant {
                        f.seen_first_assistant = true;
                        f.first_request_tokens = u.input + u.cache_read + u.cache_write;
                    }
                    f.assistant_output_tokens += u.output;
                }
                // Harvest tool_use ids so results can be named later.
                for block in content_blocks(&raw) {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        if let (Some(id), Some(name)) = (
                            block.get("id").and_then(|v| v.as_str()),
                            block.get("name").and_then(|v| v.as_str()),
                        ) {
                            f.tool_names.insert(id.to_string(), name.to_string());
                        }
                    }
                }
            }
            Some("user") => {
                if let Some(text) = raw
                    .message
                    .as_ref()
                    .and_then(|m| m.content.as_ref())
                    .and_then(|c| c.as_str())
                {
                    // String content: a typed prompt or a slash-command wrapper.
                    if text.trim_start().starts_with('<') {
                        f.wrapper_chars += text.len();
                    } else {
                        if human_prompt_text(&raw).is_some() && f.first_typed_chars == 0 {
                            f.first_typed_chars = text.len();
                        }
                        f.typed_chars += text.len();
                    }
                } else {
                    // Array content: tool_result blocks, joined to their tool.
                    for block in content_blocks(&raw) {
                        if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                            continue;
                        }
                        let name = block
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .and_then(|id| f.tool_names.get(id))
                            .cloned()
                            .unwrap_or_else(|| "(unknown tool)".to_string());
                        let chars = result_chars(block);
                        let entry = f.tools.entry(name).or_default();
                        entry.0 += 1;
                        entry.1 += chars;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(f)
}

/// The blocks of a record's `message.content` array (empty for string content).
fn content_blocks(raw: &RawLine) -> impl Iterator<Item = &serde_json::Value> {
    raw.message
        .as_ref()
        .and_then(|m| m.content.as_ref())
        .and_then(|c| c.as_array())
        .map(|a| a.iter())
        .into_iter()
        .flatten()
}

/// A human label and text size for one `attachment` record. Hooks name the
/// specific hook (that's the prunable unit); other types read as their type.
/// Size prefers the `.content` string; falls back to the serialized value.
fn attachment_label_and_size(att: &serde_json::Value) -> (String, usize) {
    let kind = att.get("type").and_then(|v| v.as_str()).unwrap_or("attachment");
    let content_len = att.get("content").and_then(|v| v.as_str()).map(str::len);
    let label = if kind == "hook_success" || kind == "hook_failure" {
        let hook = att
            .get("command")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| att.get("hookName").and_then(|v| v.as_str()))
            .unwrap_or("?");
        format!(
            "{} hook: {}",
            att.get("hookEvent").and_then(|v| v.as_str()).unwrap_or("?"),
            hook
        )
    } else {
        kind.replace('_', " ")
    };
    (label, content_len.unwrap_or_else(|| att.to_string().len()))
}

/// Total text chars inside one `tool_result` block — `content` is either a
/// plain string or an array of `{type:"text",text}` blocks.
fn result_chars(block: &serde_json::Value) -> usize {
    match block.get("content") {
        Some(serde_json::Value::String(s)) => s.len(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
            .map(str::len)
            .sum(),
        _ => 0,
    }
}

// --- hidden-base sub-attribution (disk) -------------------------------------

/// The CLAUDE.md files Claude Code loads for a session at `cwd`: the global
/// `~/.claude/CLAUDE.md`, then one per `cwd` ancestor from `home` (exclusive)
/// down to `cwd` — outermost first, matching load order. Only existing files.
fn claude_md_paths(home: &Path, cwd: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = vec![home.join(".claude").join("CLAUDE.md")];
    if let Some(cwd) = cwd {
        let ancestors: Vec<&Path> = cwd
            .ancestors()
            .take_while(|a| a.starts_with(home) && *a != home)
            .collect();
        paths.extend(ancestors.iter().rev().map(|a| a.join("CLAUDE.md")));
    }
    paths.retain(|p| p.is_file());
    paths
}

/// Re-read the un-persisted context contributors from disk: the CLAUDE.md
/// chain and the project's memory index. `(display label, est tokens)`.
fn disk_files(home: &Path, cwd: Option<&Path>, slug_dir: &Path) -> Vec<(String, u64)> {
    let mut files = claude_md_paths(home, cwd);
    let memory = slug_dir.join("memory").join("MEMORY.md");
    if memory.is_file() {
        files.push(memory);
    }
    files
        .into_iter()
        .filter_map(|p| {
            let chars = std::fs::read_to_string(&p).ok()?.len();
            let label = match p.strip_prefix(home) {
                Ok(rel) => format!("~/{}", rel.display()),
                Err(_) => p.display().to_string(),
            };
            Some((label, est_tokens(chars)))
        })
        .collect()
}

/// Exact token total across a session's `subagents/*.jsonl`: `(count, tokens)`.
fn subagent_totals(slug_dir: &Path, session_id: &str) -> (u32, u64) {
    let pattern = slug_dir.join(session_id).join("subagents").join("*.jsonl");
    let Ok(paths) = glob::glob(&pattern.to_string_lossy()) else {
        return (0, 0);
    };
    let (mut count, mut tokens) = (0u32, 0u64);
    for path in paths.flatten() {
        let Ok(file) = std::fs::File::open(&path) else { continue };
        count += 1;
        for line in std::io::BufReader::new(file).lines() {
            let Ok(line) = line else { continue };
            let Ok(raw) = serde_json::from_str::<RawLine>(&line) else {
                continue;
            };
            if let Some(u) = usage_of(&raw) {
                tokens += u.input + u.output + u.cache_read + u.cache_write;
            }
        }
    }
    (count, tokens)
}

// --- assembly + report ------------------------------------------------------

pub fn dissect(path: &Path, sessions_in_window: usize) -> Result<Dissection> {
    let f = fold_file(path)?;
    let home = dirs::home_dir().context("cannot determine home directory")?;
    let slug_dir = path.parent().context("transcript has no parent dir")?;
    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .context("transcript has no file stem")?;

    // Everything transcript-visible on turn 1: the session-start attachments
    // plus the first typed prompt. The first request's context minus that is
    // the un-persisted base (system prompt + tools + CLAUDE.md + memory).
    let visible_turn1: usize =
        f.start_attachments.values().sum::<usize>() + f.first_typed_chars;
    let hidden_base_tokens = f
        .first_request_tokens
        .saturating_sub(est_tokens(visible_turn1));

    let (subagent_count, subagent_tokens) = subagent_totals(slug_dir, &session_id);

    let mut session_start: Vec<(String, u64)> = f
        .start_attachments
        .into_iter()
        .map(|(label, chars)| (label, est_tokens(chars)))
        .collect();
    session_start.sort_by_key(|&(_, t)| std::cmp::Reverse(t));
    let mut mid_injections: Vec<(String, u64)> = f
        .mid_attachments
        .into_iter()
        .map(|(label, chars)| (label, est_tokens(chars)))
        .collect();
    mid_injections.sort_by_key(|&(_, t)| std::cmp::Reverse(t));
    let mut tools: Vec<(String, u32, u64)> = f
        .tools
        .into_iter()
        .map(|(name, (calls, chars))| (name, calls, est_tokens(chars)))
        .collect();
    tools.sort_by_key(|&(_, _, t)| std::cmp::Reverse(t));

    Ok(Dissection {
        project: transcripts::project_of(path),
        session_id,
        first_request_tokens: f.first_request_tokens,
        hidden_base_tokens,
        disk_files: disk_files(&home, f.cwd.as_ref().map(Path::new), slug_dir),
        session_start,
        tools,
        mid_injections,
        assistant_output_tokens: f.assistant_output_tokens,
        typed_tokens: est_tokens(f.typed_chars),
        command_wrapper_tokens: est_tokens(f.wrapper_chars),
        subagent_count,
        subagent_tokens,
        sessions_in_window,
    })
}

pub fn run(path: &Path, sessions_in_window: usize, days: u32, json: bool) -> Result<()> {
    let d = dissect(path, sessions_in_window)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&d)?);
        return Ok(());
    }

    let short_id: String = d.session_id.chars().take(8).collect();
    println!(
        "{short_id} · {} · first request context: {} tok (exact)\n",
        d.project,
        humanize_tokens(d.first_request_tokens)
    );

    println!("SESSION-START OVERHEAD (paid at every session start, mostly cache-write)");
    row(0, "hidden base (not persisted in transcript)", d.hidden_base_tokens, "");
    let disk_sum: u64 = d.disk_files.iter().map(|&(_, t)| t).sum();
    for (label, t) in &d.disk_files {
        row(1, label, *t, "← prunable");
    }
    row(
        1,
        "system prompt + tool defs (rest)",
        d.hidden_base_tokens.saturating_sub(disk_sum),
        "(not prunable)",
    );
    for (label, t) in &d.session_start {
        let note = if label.contains("hook") { "← prunable" } else { "" };
        row(0, label, *t, note);
    }
    println!(
        "  ── × {} sessions in the last {days}d ≈ {} tok of session-start context",
        d.sessions_in_window,
        humanize_tokens(d.first_request_tokens * d.sessions_in_window as u64)
    );

    println!("\nCONVERSATION GROWTH (this session)");
    let tool_sum: u64 = d.tools.iter().map(|&(_, _, t)| t).sum();
    row(0, "tool results", tool_sum, "");
    for (name, calls, t) in &d.tools {
        row(1, &format!("{name} ({calls} calls)"), *t, "");
    }
    for (label, t) in &d.mid_injections {
        row(0, label, *t, "");
    }
    row(0, "assistant output incl. thinking (exact)", d.assistant_output_tokens, "");
    row(0, "your typed text", d.typed_tokens, "");
    row(0, "slash-command wrappers", d.command_wrapper_tokens, "");
    if d.subagent_count > 0 {
        row(
            0,
            &format!("subagents ({} transcripts, exact)", d.subagent_count),
            d.subagent_tokens,
            "",
        );
    }
    println!("\n~ figures are chars/4 estimates; exact figures come from API usage");
    Ok(())
}

fn row(indent: usize, label: &str, tokens: u64, note: &str) {
    // Rows whose label says "(exact)" carry usage-derived numbers; everything
    // else is a chars/4 estimate and gets the ~ marker.
    let prefix = if label.contains("exact)") { "" } else { "~" };
    let pad = "  ".repeat(indent + 1);
    let label = format!("{pad}{label}");
    println!("{label:<58} {:>8}   {note}", format!("{prefix}{}", humanize_tokens(tokens)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOOK: &str = r#"{"type":"attachment","attachment":{"type":"hook_success","hookEvent":"SessionStart","hookName":"SessionStart:startup","command":"Loading ponytail mode...","content":"PONYTAIL MODE ACTIVE aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","stdout":"dup"}}"#;
    const SKILLS: &str = r#"{"type":"attachment","attachment":{"type":"skill_listing","content":"ssssssssssssssssssssssssssssssssssssssss","skillCount":4}}"#;
    const TYPED: &str = r#"{"type":"user","cwd":"/Users/x/dev/proj","message":{"role":"user","content":"dissect my tokens please dissect them well"}}"#;
    const WRAPPER: &str = r#"{"type":"user","message":{"role":"user","content":"<command-name>/model</command-name>"}}"#;
    const FIRST_ASSISTANT: &str = r#"{"type":"assistant","requestId":"req_1","message":{"id":"msg_1","model":"claude-fable-5","usage":{"input_tokens":100,"output_tokens":40,"cache_read_input_tokens":0,"cache_creation_input_tokens":900},"content":[{"type":"text","text":"hi"}]}}"#;
    const TOOL_USE: &str = r#"{"type":"assistant","requestId":"req_2","message":{"id":"msg_2","model":"claude-fable-5","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":1000,"cache_creation_input_tokens":0},"content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"/x"}}]}}"#;
    const TOOL_RESULT_STR: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr"}]}}"#;
    const TOOL_RESULT_ARR: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"tttttttttttttttttttt"},{"type":"text","text":"tttttttttttttttttttt"}]}]}}"#;
    const TASK_REMINDER: &str = r#"{"type":"attachment","attachment":{"type":"task_reminder","content":"mmmmmmmmmmmmmmmmmmmm","itemCount":2}}"#;
    const ORPHAN_RESULT: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_nope","content":"oooooooo"}]}}"#;

    fn write_session(dir: &Path, session: &str, lines: &[&str]) -> PathBuf {
        let path = dir.join(format!("{session}.jsonl"));
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    fn fold_fixture(lines: &[&str]) -> Fold {
        let dir = tempfile::tempdir().unwrap();
        let path = write_session(dir.path(), "s1", lines);
        fold_file(&path).unwrap()
    }

    #[test]
    fn attachments_split_at_first_assistant() {
        let f = fold_fixture(&[HOOK, SKILLS, TYPED, FIRST_ASSISTANT, TASK_REMINDER]);
        assert_eq!(f.start_attachments.len(), 2);
        assert!(f
            .start_attachments
            .contains_key("SessionStart hook: Loading ponytail mode..."));
        assert_eq!(f.start_attachments["skill listing"], 40);
        assert_eq!(f.mid_attachments["task reminder"], 20);
        assert_eq!(f.first_request_tokens, 1000);
        assert_eq!(f.assistant_output_tokens, 40);
    }

    #[test]
    fn tool_results_join_to_names_in_both_content_shapes() {
        let f = fold_fixture(&[
            TYPED,
            FIRST_ASSISTANT,
            TOOL_USE,
            TOOL_RESULT_STR,
            TOOL_RESULT_ARR,
            ORPHAN_RESULT,
        ]);
        assert_eq!(f.tools["Read"], (2, 80)); // 40 string + 2×20 text blocks
        assert_eq!(f.tools["(unknown tool)"], (1, 8));
    }

    #[test]
    fn typed_vs_wrapper_classification() {
        let f = fold_fixture(&[TYPED, WRAPPER, FIRST_ASSISTANT]);
        assert_eq!(f.typed_chars, 42);
        assert_eq!(f.first_typed_chars, 42);
        assert_eq!(f.wrapper_chars, 35);
        assert_eq!(f.cwd.as_deref(), Some("/Users/x/dev/proj"));
    }

    #[test]
    fn hidden_base_subtracts_visible_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let slug = dir.path().join("-Users-x-proj");
        std::fs::create_dir_all(&slug).unwrap();
        let path = write_session(&slug, "s1", &[HOOK, TYPED, FIRST_ASSISTANT]);
        let d = dissect(&path, 3).unwrap();
        // 1000 exact minus est((60-char hook)+(42-char prompt)) = 1000 - 25
        assert_eq!(d.first_request_tokens, 1000);
        assert_eq!(d.hidden_base_tokens, 975);
        assert_eq!(d.sessions_in_window, 3);
        // a tiny first request can't go negative
        let path2 = write_session(&slug, "s2", &[HOOK, TYPED]);
        let d2 = dissect(&path2, 1).unwrap();
        assert_eq!(d2.hidden_base_tokens, 0);
    }

    #[test]
    fn claude_md_walk_collects_home_global_and_cwd_chain() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("dev").join("proj").join("sub");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        for p in [
            home.join(".claude").join("CLAUDE.md"),
            home.join("dev").join("proj").join("CLAUDE.md"),
            cwd.join("CLAUDE.md"),
        ] {
            std::fs::write(p, "# notes").unwrap();
        }
        let paths = claude_md_paths(home, Some(&cwd));
        assert_eq!(paths.len(), 3);
        assert!(paths[0].ends_with(".claude/CLAUDE.md"));
        // outermost ancestor first, cwd last
        assert!(paths[1].ends_with("proj/CLAUDE.md"));
        assert!(paths[2].ends_with("sub/CLAUDE.md"));
        // cwd outside home → only the global
        assert_eq!(claude_md_paths(home, Some(Path::new("/elsewhere"))).len(), 1);
    }

    #[test]
    fn subagent_usage_is_summed_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sess1").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("agent-a.jsonl"), TOOL_USE).unwrap();
        std::fs::write(sub.join("agent-b.jsonl"), format!("{FIRST_ASSISTANT}\n{TOOL_USE}"))
            .unwrap();
        let (count, tokens) = subagent_totals(dir.path(), "sess1");
        assert_eq!(count, 2);
        // TOOL_USE = 10+5+1000 = 1015 twice; FIRST_ASSISTANT = 100+40+900 = 1040
        assert_eq!(tokens, 1015 * 2 + 1040);
        assert_eq!(subagent_totals(dir.path(), "nope"), (0, 0));
    }

    #[test]
    fn est_tokens_is_quarter_chars() {
        assert_eq!(est_tokens(100), 25);
        assert_eq!(est_tokens(3), 0);
    }
}
