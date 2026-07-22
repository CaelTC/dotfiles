//! Per-prompt token usage dissected from Claude Code's **own** transcripts
//! (`~/.claude/projects/<slug>/<sessionId>.jsonl`) — a separate data source from
//! this crate's `~/.cca` proxy store, which has no notion of a user prompt.
//!
//! Attribution is by file order: transcripts are append-only chronological, so
//! every assistant record is charged to the most recent preceding human prompt.
//! Subagent (sidechain) traffic interleaves during the prompt that spawned it and
//! is therefore charged to that prompt — the desired "this prompt cost X
//! including its subagents" semantics. `costUSD` in transcripts is always null,
//! so cost is estimated here from a hardcoded per-model price table.

use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::tui::humanize_tokens;

/// One human prompt and everything spent answering it (tool loops and subagents
/// included). Counters mirror [`crate::throughput::Throughput`]'s four fields.
#[derive(Debug, Clone, Serialize)]
pub struct PromptUsage {
    pub ts_ms: i64,
    pub prompt: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Model of the largest single attributed record ("—" if none).
    pub model: String,
    pub cost_usd: f64,
    /// True when any attributed record's model had no price table entry — the
    /// tokens still count, but `cost_usd` under-reports.
    pub unpriced: bool,
    pub requests: u32,
    #[serde(skip)]
    model_max: u64,
}

impl PromptUsage {
    fn new(ts_ms: i64, prompt: &str) -> Self {
        Self {
            ts_ms,
            prompt: prompt.to_string(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            model: "—".to_string(),
            cost_usd: 0.0,
            unpriced: false,
            requests: 0,
            model_max: 0,
        }
    }

    fn add(&mut self, u: &RecordUsage<'_>) {
        self.input += u.input;
        self.output += u.output;
        self.cache_read += u.cache_read;
        self.cache_write += u.cache_write;
        self.requests += 1;
        let total = u.input + u.output + u.cache_read + u.cache_write;
        if total >= self.model_max && !u.model.is_empty() {
            self.model = u.model.to_string();
            self.model_max = total;
        }
        match price_for(u.model) {
            Some(p) => self.cost_usd += record_cost(p, u),
            None => self.unpriced = true,
        }
    }

    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// One transcript file (`<sessionId>.jsonl`) rolled up into its prompts.
#[derive(Debug, Clone, Serialize)]
pub struct SessionUsage {
    /// The file stem — the Claude Code session id.
    pub id: String,
    /// The parent dir slug's last `-`-segment (lossy but cosmetic-only).
    pub project: String,
    pub mtime_ms: i64,
    pub prompts: Vec<PromptUsage>,
}

impl SessionUsage {
    /// Summed counters: `(input, output, cache_read, cache_write)`.
    pub fn totals(&self) -> (u64, u64, u64, u64) {
        self.prompts.iter().fold((0, 0, 0, 0), |acc, p| {
            (
                acc.0 + p.input,
                acc.1 + p.output,
                acc.2 + p.cache_read,
                acc.3 + p.cache_write,
            )
        })
    }

    pub fn total(&self) -> u64 {
        self.prompts.iter().map(PromptUsage::total).sum()
    }

    /// `(estimated USD, any record unpriced)`.
    pub fn cost(&self) -> (f64, bool) {
        self.prompts.iter().fold((0.0, false), |acc, p| {
            (acc.0 + p.cost_usd, acc.1 || p.unpriced)
        })
    }
}

// --- line-level parsing -----------------------------------------------------

/// Permissive per-line shape: all `Option`s, so the many bookkeeping record
/// types (`summary`, `file-history-snapshot`, …) and malformed lines fall out
/// naturally — same tolerance as `store::read_records`.
#[derive(Deserialize)]
pub(crate) struct RawLine {
    #[serde(rename = "type")]
    pub(crate) kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "isMeta")]
    is_meta: Option<bool>,
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
    #[serde(rename = "requestId")]
    pub(crate) request_id: Option<String>,
    pub(crate) cwd: Option<String>,
    /// Claude Code's injected-context records (`type:"attachment"`) —
    /// heterogeneous per `.type`, so probed as a `Value` rather than schemed.
    pub(crate) attachment: Option<serde_json::Value>,
    pub(crate) message: Option<RawMessage>,
}

#[derive(Deserialize)]
pub(crate) struct RawMessage {
    pub(crate) id: Option<String>,
    model: Option<String>,
    /// A string for a typed human prompt; an array for tool_results/blocks.
    pub(crate) content: Option<serde_json::Value>,
    usage: Option<RawUsage>,
}

/// Top-level counters only. The nested `cache_creation` breakdown and the
/// `iterations[]` array are deliberately ignored — iterations DUPLICATE the
/// top-level numbers and would double-count.
#[derive(Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

/// The prompt text if this line is a genuine typed human prompt. String content
/// distinguishes it from tool_results (arrays); `isMeta` filters injected
/// caveats; `isSidechain` filters subagent kickoff prompts (also plain strings —
/// without this every Task spawn would start a phantom prompt bucket); a `<`
/// prefix filters slash-command wrappers like `<command-name>`.
pub(crate) fn human_prompt_text(raw: &RawLine) -> Option<&str> {
    if raw.kind.as_deref() != Some("user")
        || raw.is_meta.unwrap_or(false)
        || raw.is_sidechain.unwrap_or(false)
    {
        return None;
    }
    let text = raw.message.as_ref()?.content.as_ref()?.as_str()?;
    if text.trim_start().starts_with('<') {
        return None;
    }
    Some(text)
}

pub(crate) struct RecordUsage<'a> {
    pub(crate) model: &'a str,
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
}

/// The token counters if this line is a countable assistant record.
/// `<synthetic>` records (limit banners etc.) are filtered by name.
pub(crate) fn usage_of(raw: &RawLine) -> Option<RecordUsage<'_>> {
    if raw.kind.as_deref() != Some("assistant") {
        return None;
    }
    let msg = raw.message.as_ref()?;
    let model = msg.model.as_deref().unwrap_or("");
    if model == "<synthetic>" {
        return None;
    }
    let u = msg.usage.as_ref()?;
    Some(RecordUsage {
        model,
        input: u.input_tokens.unwrap_or(0),
        output: u.output_tokens.unwrap_or(0),
        cache_read: u.cache_read_input_tokens.unwrap_or(0),
        cache_write: u.cache_creation_input_tokens.unwrap_or(0),
    })
}

fn ts_ms(raw: &RawLine) -> i64 {
    raw.timestamp
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}

// --- bucketing --------------------------------------------------------------

/// Stream one transcript and fold it into per-prompt buckets. `seen` dedupes
/// assistant records by `(message.id, requestId)` across ALL files: resumed
/// sessions copy prior records into a new file, and without this every resume
/// double-counts. Process files newest-mtime-first so usage attributes to the
/// most recent copy. Records missing either id always count. Returns `None` for
/// unreadable files and pure-bookkeeping files (no prompts, no usage).
pub fn parse_session(
    path: &Path,
    mtime_ms: i64,
    seen: &mut HashSet<(String, String)>,
) -> Option<SessionUsage> {
    let file = std::fs::File::open(path).ok()?;
    let mut prompts: Vec<PromptUsage> = Vec::new();
    let mut current: Option<PromptUsage> = None;
    for line in std::io::BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        let Ok(raw) = serde_json::from_str::<RawLine>(&line) else {
            continue;
        };
        if let Some(text) = human_prompt_text(&raw) {
            prompts.extend(current.take());
            current = Some(PromptUsage::new(ts_ms(&raw), text));
        } else if let Some(u) = usage_of(&raw) {
            let msg_id = raw.message.as_ref().and_then(|m| m.id.clone());
            if let (Some(mid), Some(rid)) = (msg_id, raw.request_id.clone()) {
                if !seen.insert((mid, rid)) {
                    continue;
                }
            }
            // Usage before any prompt (resumed/compacted session) lands in a
            // placeholder bucket so session totals stay truthful.
            current
                .get_or_insert_with(|| PromptUsage::new(ts_ms(&raw), "(continuation)"))
                .add(&u);
        }
    }
    prompts.extend(current.take());
    if prompts.is_empty() {
        return None;
    }
    Some(SessionUsage {
        id: path.file_stem().map(|s| s.to_string_lossy().into_owned())?,
        project: project_of(path),
        mtime_ms,
        prompts,
    })
}

/// Prettify the parent dir slug (`-Users-macbook-dev-dotfiles-claude-dash`) to
/// its last segment. Lossy for dashed real names; cosmetic only.
pub(crate) fn project_of(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy())
        .and_then(|slug| slug.rsplit('-').next().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".to_string())
}

// --- discovery --------------------------------------------------------------

pub fn transcripts_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine home directory")?
        .join(".claude")
        .join("projects"))
}

fn is_recent(mtime_ms: i64, now_ms: i64, days: u32) -> bool {
    now_ms - mtime_ms <= i64::from(days) * 86_400_000
}

/// Transcript files under `dir` modified in the last `days` days, newest first
/// (the order [`parse_session`]'s dedupe relies on).
pub fn recent_session_files(dir: &Path, days: u32, now_ms: i64) -> Vec<(PathBuf, i64)> {
    let pattern = dir.join("*").join("*.jsonl");
    let mut files: Vec<(PathBuf, i64)> = match glob::glob(&pattern.to_string_lossy()) {
        Ok(paths) => paths
            .flatten()
            .filter_map(|p| {
                let mtime = p
                    .metadata()
                    .ok()?
                    .modified()
                    .ok()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?
                    .as_millis() as i64;
                is_recent(mtime, now_ms, days).then_some((p, mtime))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort_by_key(|&(_, m)| std::cmp::Reverse(m));
    files
}

/// Parse every recent transcript into [`SessionUsage`]s, newest first.
pub fn load_recent(days: u32) -> Result<Vec<SessionUsage>> {
    let dir = transcripts_dir()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut seen = HashSet::new();
    Ok(recent_session_files(&dir, days, now_ms)
        .into_iter()
        .filter_map(|(p, m)| parse_session(&p, m, &mut seen))
        .collect())
}

// --- pricing ----------------------------------------------------------------

/// USD per MTok.
struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

/// Prices as of 2026-07 — see <https://platform.claude.com/docs/en/pricing>.
/// Prefix match, first match wins; keep more-specific prefixes first.
/// Documented approximations: all cache writes are priced at the 5-minute rate
/// (1h writes cost 2× — slight underestimate; `cache_creation.
/// ephemeral_1h_input_tokens` exists in transcripts if precision is ever
/// wanted), and Sonnet 5's introductory pricing is ignored for sticker price.
const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5", Price { input: 10.0, output: 50.0, cache_read: 1.00, cache_write: 12.50 }),
    ("claude-mythos", Price { input: 10.0, output: 50.0, cache_read: 1.00, cache_write: 12.50 }),
    ("claude-opus-4", Price { input: 5.0, output: 25.0, cache_read: 0.50, cache_write: 6.25 }),
    ("claude-sonnet-5", Price { input: 3.0, output: 15.0, cache_read: 0.30, cache_write: 3.75 }),
    ("claude-sonnet-4", Price { input: 3.0, output: 15.0, cache_read: 0.30, cache_write: 3.75 }),
    ("claude-haiku-4", Price { input: 1.0, output: 5.0, cache_read: 0.10, cache_write: 1.25 }),
];

fn price_for(model: &str) -> Option<&'static Price> {
    PRICES
        .iter()
        .find(|(prefix, _)| model.starts_with(prefix))
        .map(|(_, p)| p)
}

fn record_cost(p: &Price, u: &RecordUsage<'_>) -> f64 {
    (u.input as f64 * p.input
        + u.output as f64 * p.output
        + u.cache_read as f64 * p.cache_read
        + u.cache_write as f64 * p.cache_write)
        / 1e6
}

/// Render an estimated cost: `<1¢` under a cent, `+?` appended when unpriced
/// records mean the number under-reports, bare `?` when nothing was priceable.
pub fn format_cost(cost_usd: f64, unpriced: bool) -> String {
    if unpriced && cost_usd == 0.0 {
        return "?".to_string();
    }
    let base = if cost_usd > 0.0 && cost_usd < 0.01 {
        "<1¢".to_string()
    } else {
        format!("${cost_usd:.2}")
    };
    if unpriced {
        format!("{base}+?")
    } else {
        base
    }
}

// --- CLI rendering ----------------------------------------------------------

/// `claude-dash tokens` — overview of recent sessions, a per-prompt drill-down
/// with `--session <id-prefix>`, or a context-composition report with
/// `--dissect`; `--json` emits the parsed data.
pub fn run(days: u32, session: Option<String>, json: bool, dissect: bool) -> Result<()> {
    if dissect {
        // Dissect works on the raw file, not the per-prompt rollup: resolve the
        // session (newest by default) straight from discovery.
        let dir = transcripts_dir()?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let files = recent_session_files(&dir, days, now_ms);
        let matches: Vec<&(PathBuf, i64)> = match &session {
            Some(prefix) => files
                .iter()
                .filter(|(p, _)| {
                    p.file_stem()
                        .map(|s| s.to_string_lossy().starts_with(prefix.as_str()))
                        .unwrap_or(false)
                })
                .collect(),
            None => files.first().into_iter().collect(),
        };
        return match matches.as_slice() {
            [] => bail!("no session in the last {days} days to dissect"),
            [(path, _)] => crate::dissect::run(path, files.len(), days, json),
            many => bail!(
                "ambiguous session prefix: {}",
                many.iter()
                    .filter_map(|(p, _)| p.file_stem())
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
    }
    let mut sessions = load_recent(days)?;
    if let Some(prefix) = session {
        let matches: Vec<usize> = sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.id.starts_with(&prefix))
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [] => bail!("no session in the last {days} days matches '{prefix}'"),
            [i] => {
                let s = sessions.swap_remove(*i);
                if json {
                    println!("{}", serde_json::to_string_pretty(&s)?);
                } else {
                    print_drilldown(&s);
                }
            }
            many => bail!(
                "'{prefix}' is ambiguous: {}",
                many.iter()
                    .map(|&i| sessions[i].id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    } else if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
    } else {
        print_overview(&sessions, days);
    }
    Ok(())
}

/// First line of the prompt, control chars stripped, truncated to `width`.
fn clean_prompt(text: &str, width: usize) -> String {
    let cleaned: String = text
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if cleaned.chars().count() > width {
        let mut s: String = cleaned.chars().take(width.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        cleaned
    }
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

fn print_overview(sessions: &[SessionUsage], days: u32) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    println!(
        "{:<9} {:<18} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9}  LAST",
        "SESSION", "PROJECT", "PROMPTS", "IN", "OUT", "CACHE-R", "CACHE-W", "TOTAL", "COST"
    );
    let mut sums = (0u64, 0u64, 0u64, 0u64);
    let mut prompt_count = 0usize;
    let (mut cost_sum, mut any_unpriced) = (0.0f64, false);
    for s in sessions {
        let (i, o, cr, cw) = s.totals();
        let (cost, unpriced) = s.cost();
        sums = (sums.0 + i, sums.1 + o, sums.2 + cr, sums.3 + cw);
        prompt_count += s.prompts.len();
        cost_sum += cost;
        any_unpriced |= unpriced;
        println!(
            "{:<9} {:<18} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9}  {}",
            short_id(&s.id),
            clean_prompt(&s.project, 18),
            s.prompts.len(),
            humanize_tokens(i),
            humanize_tokens(o),
            humanize_tokens(cr),
            humanize_tokens(cw),
            humanize_tokens(s.total()),
            format_cost(cost, unpriced),
            lifecycle_ago(s.mtime_ms, now_ms),
        );
    }
    println!(
        "{:<9} {:<18} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9}",
        "TOTAL",
        format!("(last {days}d)"),
        prompt_count,
        humanize_tokens(sums.0),
        humanize_tokens(sums.1),
        humanize_tokens(sums.2),
        humanize_tokens(sums.3),
        humanize_tokens(sums.0 + sums.1 + sums.2 + sums.3),
        format_cost(cost_sum, any_unpriced),
    );
    println!("costs are API-equivalent estimates (subscription usage is not billed per token)");
}

fn print_drilldown(s: &SessionUsage) {
    let (cost, unpriced) = s.cost();
    println!(
        "{} · {} · {} prompts · {} tok · {}",
        short_id(&s.id),
        s.project,
        s.prompts.len(),
        humanize_tokens(s.total()),
        format_cost(cost, unpriced),
    );
    println!(
        "{:<12} {:<42} {:<16} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9}",
        "TIME", "PROMPT", "MODEL", "IN", "OUT", "CACHE-R", "CACHE-W", "TOTAL", "COST"
    );
    for p in &s.prompts {
        println!(
            "{:<12} {:<42} {:<16} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9}",
            prompt_time(p.ts_ms),
            clean_prompt(&p.prompt, 42),
            p.model.strip_prefix("claude-").unwrap_or(&p.model),
            humanize_tokens(p.input),
            humanize_tokens(p.output),
            humanize_tokens(p.cache_read),
            humanize_tokens(p.cache_write),
            humanize_tokens(p.total()),
            format_cost(p.cost_usd, p.unpriced),
        );
    }
    println!("costs are API-equivalent estimates (subscription usage is not billed per token)");
}

/// `MM-DD HH:MM` in local time; `—` for the missing-timestamp fallback.
fn prompt_time(ts_ms: i64) -> String {
    use chrono::TimeZone;
    if ts_ms == 0 {
        return "—".to_string();
    }
    match chrono::Local.timestamp_millis_opt(ts_ms) {
        chrono::LocalResult::Single(t) => t.format("%m-%d %H:%M").to_string(),
        _ => "—".to_string(),
    }
}

fn lifecycle_ago(mtime_ms: i64, now_ms: i64) -> String {
    crate::lifecycle::format_ended_ago(mtime_ms, now_ms)
}

/// A small in-memory [`SessionUsage`] fixture shared with the TUI's
/// `tokens_lines` test.
#[cfg(test)]
pub fn test_sessions() -> Vec<SessionUsage> {
    let mut a = PromptUsage::new(1_000, "fix the flaky test");
    a.add(&RecordUsage { model: "claude-opus-4-8", input: 100, output: 10, cache_read: 1_000, cache_write: 50 });
    let b = PromptUsage::new(2_000, "now update the README");
    vec![SessionUsage {
        id: "abcd1234-5678".to_string(),
        project: "proj".to_string(),
        mtime_ms: 42,
        prompts: vec![a, b],
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(line: &str) -> RawLine {
        serde_json::from_str(line).unwrap()
    }

    const PROMPT_A: &str = r#"{"type":"user","timestamp":"2026-07-19T14:02:00.000Z","message":{"role":"user","content":"fix the flaky test"}}"#;
    const PROMPT_B: &str = r#"{"type":"user","timestamp":"2026-07-19T14:31:00.000Z","message":{"role":"user","content":"now update the README"}}"#;
    const ASSIST_1: &str = r#"{"type":"assistant","timestamp":"2026-07-19T14:02:10.000Z","requestId":"req_1","message":{"id":"msg_1","model":"claude-opus-4-8","usage":{"input_tokens":100,"output_tokens":10,"cache_read_input_tokens":1000,"cache_creation_input_tokens":50}}}"#;
    const ASSIST_SIDECHAIN: &str = r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-07-19T14:03:00.000Z","requestId":"req_2","message":{"id":"msg_2","model":"claude-haiku-4-5-20251001","usage":{"input_tokens":20,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
    const ASSIST_3: &str = r#"{"type":"assistant","timestamp":"2026-07-19T14:32:00.000Z","requestId":"req_3","message":{"id":"msg_3","model":"claude-opus-4-8","usage":{"input_tokens":200,"output_tokens":20,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;

    fn parse_lines(lines: &[&str]) -> Option<SessionUsage> {
        parse_lines_seen(lines, &mut HashSet::new())
    }

    fn parse_lines_seen(
        lines: &[&str],
        seen: &mut HashSet<(String, String)>,
    ) -> Option<SessionUsage> {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-Users-x-proj-a");
        std::fs::create_dir_all(&proj).unwrap();
        let path = proj.join("abcd1234-5678.jsonl");
        std::fs::write(&path, lines.join("\n")).unwrap();
        parse_session(&path, 42, seen)
    }

    #[test]
    fn human_prompt_predicate() {
        assert!(human_prompt_text(&raw(PROMPT_A)).is_some());
        // tool_result: array content
        let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#;
        assert!(human_prompt_text(&raw(tool)).is_none());
        let meta = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"caveat"}}"#;
        assert!(human_prompt_text(&raw(meta)).is_none());
        // subagent kickoff prompts are string-content user records too — the
        // phantom-prompt regression case
        let side = r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"explore the repo"}}"#;
        assert!(human_prompt_text(&raw(side)).is_none());
        let slash = r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#;
        assert!(human_prompt_text(&raw(slash)).is_none());
    }

    #[test]
    fn bucketing_attributes_in_file_order() {
        let s = parse_lines(&[PROMPT_A, ASSIST_1, ASSIST_SIDECHAIN, PROMPT_B, ASSIST_3]).unwrap();
        assert_eq!(s.prompts.len(), 2);
        let a = &s.prompts[0];
        // sidechain usage charged to the prompt that spawned it
        assert_eq!((a.input, a.output, a.cache_read, a.cache_write), (120, 15, 1000, 50));
        assert_eq!(a.requests, 2);
        assert_eq!(a.model, "claude-opus-4-8"); // largest record's model
        let b = &s.prompts[1];
        assert_eq!((b.input, b.output), (200, 20));
        assert_eq!(s.total(), 120 + 15 + 1000 + 50 + 220);
        assert_eq!(s.id, "abcd1234-5678");
        assert_eq!(s.project, "a");
        assert_eq!(s.mtime_ms, 42);
    }

    #[test]
    fn synthetic_and_bookkeeping_skipped() {
        let synthetic = r#"{"type":"assistant","message":{"model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0}}}"#;
        let summary = r#"{"type":"summary","summary":"stuff"}"#;
        let snapshot = r#"{"type":"file-history-snapshot","messageId":"x"}"#;
        let s = parse_lines(&[PROMPT_A, synthetic, summary, snapshot, "{oops", ASSIST_1]).unwrap();
        assert_eq!(s.prompts.len(), 1);
        assert_eq!(s.prompts[0].requests, 1);
        assert_eq!(s.total(), 1160);
    }

    #[test]
    fn orphan_records_get_continuation_bucket() {
        let s = parse_lines(&[ASSIST_1, PROMPT_B, ASSIST_3]).unwrap();
        assert_eq!(s.prompts.len(), 2);
        assert_eq!(s.prompts[0].prompt, "(continuation)");
        assert_eq!(s.prompts[0].total(), 1160);
    }

    #[test]
    fn pure_bookkeeping_file_is_skipped() {
        assert!(parse_lines(&[r#"{"type":"summary","summary":"x"}"#]).is_none());
    }

    #[test]
    fn dedupe_by_message_and_request_id() {
        let mut seen = HashSet::new();
        let first = parse_lines_seen(&[PROMPT_A, ASSIST_1], &mut seen).unwrap();
        assert_eq!(first.total(), 1160);
        // a resumed session replays the same record: not counted again
        let resumed = parse_lines_seen(&[PROMPT_A, ASSIST_1], &mut seen).unwrap();
        assert_eq!(resumed.total(), 0);
        // records missing ids always count
        let no_ids = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":7,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        let a = parse_lines_seen(&[PROMPT_A, no_ids], &mut seen).unwrap();
        let b = parse_lines_seen(&[PROMPT_A, no_ids], &mut seen).unwrap();
        assert_eq!(a.total() + b.total(), 14);
    }

    #[test]
    fn price_prefix_matching() {
        assert!((price_for("claude-opus-4-8").unwrap().input - 5.0).abs() < f64::EPSILON);
        assert!((price_for("claude-haiku-4-5-20251001").unwrap().input - 1.0).abs() < f64::EPSILON);
        assert!((price_for("claude-fable-5").unwrap().output - 50.0).abs() < f64::EPSILON);
        assert!(price_for("weird-model").is_none());
        assert!(price_for("").is_none());
    }

    #[test]
    fn cost_math() {
        let p = price_for("claude-opus-4-8").unwrap();
        let u = RecordUsage { model: "claude-opus-4-8", input: 1_000_000, output: 1_000_000, cache_read: 0, cache_write: 0 };
        assert!((record_cost(p, &u) - 30.0).abs() < 1e-9);
        let u2 = RecordUsage { model: "claude-opus-4-8", input: 0, output: 0, cache_read: 1_000_000, cache_write: 1_000_000 };
        assert!((record_cost(p, &u2) - 6.75).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_counts_tokens_but_flags_unpriced() {
        let weird = r#"{"type":"assistant","message":{"model":"weird-model","usage":{"input_tokens":9,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        let s = parse_lines(&[PROMPT_A, weird]).unwrap();
        assert_eq!(s.prompts[0].total(), 10);
        assert!(s.prompts[0].unpriced);
        assert_eq!(s.prompts[0].cost_usd, 0.0);
    }

    #[test]
    fn cost_formatting() {
        assert_eq!(format_cost(4.116, false), "$4.12");
        assert_eq!(format_cost(0.004, false), "<1¢");
        assert_eq!(format_cost(0.0, false), "$0.00");
        assert_eq!(format_cost(4.12, true), "$4.12+?");
        assert_eq!(format_cost(0.0, true), "?");
    }

    #[test]
    fn recency_filter_pure() {
        let day = 86_400_000;
        assert!(is_recent(100 * day, 100 * day, 7));
        assert!(is_recent(93 * day, 100 * day, 7));
        assert!(!is_recent(92 * day, 100 * day, 7));
    }

    #[test]
    fn discovery_globs_and_sorts_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for (proj, name) in [("-x-proj-a", "s1.jsonl"), ("-x-proj-b", "s2.jsonl")] {
            let d = dir.path().join(proj);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(name), PROMPT_A).unwrap();
        }
        std::fs::write(dir.path().join("-x-proj-a").join("not-jsonl.txt"), "x").unwrap();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let files = recent_session_files(dir.path(), 7, now_ms);
        assert_eq!(files.len(), 2);
        assert!(files[0].1 >= files[1].1);
    }

    #[test]
    fn clean_prompt_truncates_and_strips() {
        assert_eq!(clean_prompt("hello\nworld", 40), "hello");
        assert_eq!(clean_prompt("a\tb", 40), "ab");
        let long = "x".repeat(50);
        let out = clean_prompt(&long, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }
}
