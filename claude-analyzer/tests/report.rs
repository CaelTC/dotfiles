//! One aggregation over the synthetic corpus in `tests/fixtures`.
//!
//! `now` is fixed so the fixtures' timestamps stay inside the Week forever.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::Path;

/// Expected in-window fresh totals, per category.
const PROMPT: i64 = 6 + 4 + 4 + 2 + 3; // msg_prompt, msg_after, msg_skilltool, msg_mcpcall, msg_s2
const TOOL: i64 = 600 + 10; // msg_tool, msg_skill2
const PLAYWRIGHT: i64 = 6000 + 5; // msg_mcp, msg_mcppw
const GMAIL: i64 = 10; // msg_mcpmulti — mixed servers, first result wins
const MCP: i64 = PLAYWRIGHT + GMAIL;
const SKILL: i64 = 15 + 50; // msg_skill1, msg_skillbody
const AGENT: i64 = 1000;
const OVERHEAD: i64 = 60;
const FRESH: i64 = PROMPT + TOOL + MCP + SKILL + AGENT + OVERHEAD;
/// Fresh tokens of the two `simplify` spans, main conversation only.
const SIMPLIFY_MAIN: i64 = 15 + 10 + 50;
/// The one request the fixtures bill to haiku, plus the subagent's.
const HAIKU: i64 = 60 + AGENT;

/// The Previous Week lives entirely in `sess-prev-0003`, a session that has no
/// current-Week traffic at all.
const PREV_PROMPT: i64 = 100 + 40; // the second lands on the boundary date
const PREV_SIMPLIFY: i64 = 200;
const PREV_MCP: i64 = 300; // the only `linear` and `claude-opus-4-8` usage
const PREV_HANDOFF: i64 = 50; // a skill that ran only in the Previous Week
const PREV_FRESH: i64 = PREV_PROMPT + PREV_SIMPLIFY + PREV_MCP + PREV_HANDOFF;

/// Rows of `days` belonging to one window.
fn window<'a>(r: &'a Value, week: &str) -> Vec<&'a Value> {
    r["days"].as_array().unwrap().iter().filter(|d| d["week"] == week).collect()
}

fn category<'a>(r: &'a Value, name: &str) -> &'a Value {
    r["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no category named {name}"))
}

fn report() -> Value {
    let now: DateTime<Utc> = "2026-01-08T12:00:00Z".parse().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    claude_analyzer::analyze(&root, now)
}

fn n(v: &Value) -> i64 {
    v.as_i64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn sum(rows: &Value, key: &str) -> i64 {
    rows.as_array().unwrap().iter().map(|r| n(&r[key])).sum()
}

#[test]
fn duplicate_message_ids_are_counted_once() {
    let r = report();
    // 12 unique main requests + 1 agent request + 1 in the second session; the
    // transcripts repeat some of those ids and add a synthetic record.
    assert_eq!(n(&r["totals"]["requests"]), 14);
    assert_eq!(n(&r["totals"]["fresh"]), FRESH);
    assert_eq!(n(&r["totals"]["cacheRead"]), 13286);
    assert_eq!(n(&r["totals"]["sessions"]), 2);
}

#[test]
fn malformed_and_unknown_lines_are_skipped_not_fatal() {
    assert_eq!(n(&report()["skippedRecords"]), 2);
}

#[test]
fn every_grouping_sums_to_totals() {
    let r = report();
    let fresh = n(&r["totals"]["fresh"]);
    let cache = n(&r["totals"]["cacheRead"]);
    for g in ["categories", "sessions", "skills", "models"] {
        assert_eq!(sum(&r[g], "fresh"), fresh, "{g} fresh");
        assert_eq!(sum(&r[g], "cacheRead"), cache, "{g} cacheRead");
    }
    // `days` spans both windows, so only its current rows answer to `totals`.
    let cur = window(&r, "current");
    assert_eq!(cur.iter().map(|d| n(&d["fresh"])).sum::<i64>(), fresh);
    assert_eq!(cur.iter().map(|d| n(&d["cacheRead"])).sum::<i64>(), cache);
    assert_eq!(sum(&r["categories"], "requests"), n(&r["totals"]["requests"]));
    assert_eq!(sum(&r["sessions"], "requests"), n(&r["totals"]["requests"]));
    assert_eq!(sum(&r["models"], "requests"), n(&r["totals"]["requests"]));
}

/// The Previous Week is a comparison line, not a second report: it contributes
/// `prevFresh` to every grouping except `sessions`.
#[test]
fn every_grouping_sums_to_total_prev_fresh() {
    let r = report();
    assert_eq!(n(&r["totals"]["prevFresh"]), PREV_FRESH);
    for g in ["categories", "models", "skills"] {
        assert_eq!(sum(&r[g], "prevFresh"), PREV_FRESH, "{g} prevFresh");
    }
    let mcp = category(&r, "MCP");
    assert_eq!(sum(&mcp["servers"], "prevFresh"), n(&mcp["prevFresh"]));
    // Sessions stay current-Week only — no prevFresh, and the Previous Week's
    // own session is absent even though its tokens are counted.
    assert_eq!(n(&r["totals"]["sessions"]), 2);
    for s in r["sessions"].as_array().unwrap() {
        assert!(s["prevFresh"].is_null(), "{} carries prevFresh", s["id"]);
        assert_ne!(s["id"], "sess-pre");
    }
}

#[test]
fn previous_week_starts_seven_days_before_the_week() {
    let r = report();
    assert_eq!(r["prevWeekStart"], "2025-12-25T12:00:00Z");
    assert_eq!(r["weekStart"], "2026-01-01T12:00:00Z");
    assert_eq!(r["weekEnd"], "2026-01-08T12:00:00Z");
}

/// Rows present in the Previous Week but gone from this one must still show up,
/// zeroed, or a disappearance would look like it never happened.
#[test]
fn rows_that_only_existed_in_the_previous_week_still_appear() {
    let r = report();
    let find = |g: &str, name: &str| -> Value {
        r[g].as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap_or_else(|| panic!("no {g} row named {name}"))
            .clone()
    };
    for (grouping, name, prev) in [
        ("models", "claude-opus-4-8", PREV_MCP),
        ("skills", "handoff", PREV_HANDOFF),
    ] {
        let row = find(grouping, name);
        assert_eq!(n(&row["fresh"]), 0, "{name} fresh");
        assert_eq!(n(&row["cacheRead"]), 0, "{name} cacheRead");
        assert_eq!(n(&row["requests"]), 0, "{name} requests");
        assert_eq!(n(&row["prevFresh"]), prev, "{name} prevFresh");
    }
    // A skill seen only in the Previous Week has no runs to report.
    assert_eq!(n(&find("skills", "handoff")["runs"]), 0);
    assert_eq!(n(&find("skills", "handoff")["avgFreshPerRun"]), 0);
    // The MCP server it used is gone from this Week too.
    let linear = category(&r, "MCP")["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "linear")
        .unwrap()
        .clone();
    assert_eq!(n(&linear["fresh"]), 0);
    assert_eq!(n(&linear["prevFresh"]), PREV_MCP);
}

/// Skill runs and per-run averages describe this Week only; the Previous Week
/// contributes tokens.
#[test]
fn previous_week_tokens_do_not_change_run_counts() {
    let r = report();
    let simplify = r["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "simplify")
        .unwrap();
    assert_eq!(n(&simplify["runs"]), 2);
    assert_eq!(n(&simplify["fresh"]), SIMPLIFY_MAIN + AGENT);
    assert_eq!(n(&simplify["avgFreshPerRun"]), (SIMPLIFY_MAIN + AGENT) / 2);
    assert_eq!(n(&simplify["prevFresh"]), PREV_SIMPLIFY);
}

#[test]
fn models_are_verbatim_and_include_subagent_usage() {
    let r = report();
    let rows = r["models"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    // Heaviest first: fable carries the main conversation, haiku the subagent,
    // and the Previous-Week-only model sorts last on zero current tokens.
    assert_eq!(rows[0]["name"], "claude-fable-5");
    assert_eq!(n(&rows[0]["fresh"]), FRESH - HAIKU);
    assert_eq!(n(&rows[0]["prevFresh"]), PREV_PROMPT + PREV_SIMPLIFY + PREV_HANDOFF);
    assert_eq!(rows[1]["name"], "claude-haiku-4-5");
    assert_eq!(n(&rows[1]["fresh"]), HAIKU);
    assert_eq!(n(&rows[1]["requests"]), 2);
    assert_eq!(rows[2]["name"], "claude-opus-4-8");
}

/// A request triggered by results from two servers at once belongs to the first
/// one in the results, whole — never apportioned.
#[test]
fn the_mcp_row_breaks_down_by_server() {
    let r = report();
    let mcp = category(&r, "MCP");
    let servers = mcp["servers"].as_array().unwrap();
    assert_eq!(sum(&mcp["servers"], "fresh"), n(&mcp["fresh"]));
    assert_eq!(sum(&mcp["servers"], "cacheRead"), n(&mcp["cacheRead"]));
    assert_eq!(sum(&mcp["servers"], "requests"), n(&mcp["requests"]));
    assert_eq!(servers[0]["name"], "playwright");
    assert_eq!(n(&servers[0]["fresh"]), PLAYWRIGHT);
    assert_eq!(servers[1]["name"], "claude_ai_Gmail");
    assert_eq!(n(&servers[1]["fresh"]), GMAIL);
    // The breakdown belongs to MCP alone.
    for c in r["categories"].as_array().unwrap() {
        assert_eq!(c["servers"].is_null(), c["name"] != "MCP", "{}", c["name"]);
    }
}

#[test]
fn requests_land_in_the_expected_categories() {
    let r = report();
    let by = |name: &str| n(&category(&r, name)["fresh"]);
    assert_eq!(by("Prompt"), PROMPT);
    assert_eq!(by("Tool call"), TOOL);
    assert_eq!(by("MCP"), MCP);
    assert_eq!(by("Skill"), SKILL);
    assert_eq!(by("Agent"), AGENT);
    assert_eq!(by("Overhead"), OVERHEAD);
}

#[test]
fn agents_nest_under_the_session_that_spawned_them() {
    let r = report();
    let s = &r["sessions"][0]; // sorted by fresh desc
    assert_eq!(s["id"], "sess-mai");
    assert_eq!(s["project"], "dev/demo");
    assert_eq!(s["firstPrompt"], "Add a login page");
    assert_eq!(n(&s["byCategory"]["Agent"]), AGENT);
    assert_eq!(n(&s["fresh"]), FRESH - 3); // everything but the second session
    let a = &s["agents"][0];
    assert_eq!(a["id"], "aaa11122");
    assert_eq!(a["model"], "claude-haiku-4-5");
    assert_eq!(n(&a["fresh"]), AGENT);
    assert_eq!(n(&a["requests"]), 1);
}

#[test]
fn windows_cwds_decode_like_unix_ones() {
    use claude_analyzer::parse::project_from_cwd;
    assert_eq!(project_from_cwd(r"C:\Users\colleague\dev\proj"), "dev/proj");
    assert_eq!(project_from_cwd(r"c:\Users\colleague\dev\proj"), "dev/proj");
    assert_eq!(project_from_cwd("/Users/me/dev/proj"), "dev/proj");
    assert_eq!(project_from_cwd("/home/me/dev/proj"), "dev/proj");
}

#[test]
fn project_falls_back_to_the_flattened_directory_name() {
    let r = report();
    let s = r["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "sess-two")
        .unwrap();
    assert_eq!(s["project"], "Documents/notes");
}

/// A request whose trigger sits two user records back — a Skill tool_result
/// followed by the injected skill body — still counts as Skill.
#[test]
fn the_strongest_trigger_since_the_last_request_wins() {
    let r = report();
    assert_eq!(n(&category(&r, "Skill")["requests"]), 2);
}

#[test]
fn consecutive_skill_spans_are_separate_runs_and_own_their_agents() {
    let r = report();
    let rows = r["skills"].as_array().unwrap();
    let simplify = rows.iter().find(|s| s["name"] == "simplify").unwrap();
    assert_eq!(n(&simplify["runs"]), 2); // broken by two skill-less requests
    assert_eq!(n(&simplify["composition"]["main"]), SIMPLIFY_MAIN);
    assert_eq!(n(&simplify["composition"]["agents"]), AGENT);
    assert_eq!(n(&simplify["fresh"]), SIMPLIFY_MAIN + AGENT);
    assert_eq!(n(&simplify["avgFreshPerRun"]), (SIMPLIFY_MAIN + AGENT) / 2);
    // The residual row closes the grouping and comes last.
    assert_eq!(rows.last().unwrap()["name"], "(no skill)");
}

#[test]
fn days_are_local_calendar_days_with_only_the_oldest_partial() {
    let r = report();
    let days = r["days"].as_array().unwrap();
    // 14 or 15 calendar dates, plus a second row for the shared boundary date.
    assert!((14..=16).contains(&days.len()), "got {} days", days.len());
    assert_eq!(days[0]["partial"], true);
    assert!(days[1..].iter().all(|d| d["partial"] == false));
    let dates: Vec<&str> = days.iter().map(|d| d["date"].as_str().unwrap()).collect();
    let mut sorted = dates.clone();
    sorted.sort();
    assert_eq!(dates, sorted, "oldest first");
    for d in days {
        let cats: i64 = ["Prompt", "Tool call", "MCP", "Skill", "Agent", "Overhead"]
            .iter()
            .map(|c| n(&d["byCategory"][c]))
            .sum();
        assert_eq!(cats, n(&d["fresh"]), "{} byCategory", d["date"]);
    }
}

/// Each day belongs to exactly one window, previous days first, and the two
/// halves answer to their own totals.
#[test]
fn days_are_tagged_by_window_and_the_boundary_day_is_current() {
    let r = report();
    let days = r["days"].as_array().unwrap();
    let tags: Vec<&str> = days.iter().map(|d| d["week"].as_str().unwrap()).collect();
    assert!(tags.iter().all(|w| *w == "previous" || *w == "current"));
    // No interleaving: every "previous" row precedes every "current" one.
    let first_current = tags.iter().position(|w| *w == "current").unwrap();
    assert!(tags[first_current..].iter().all(|w| *w == "current"));
    assert!(tags[..first_current].iter().all(|w| *w == "previous"));
    // The boundary day is the Week's own start date, tagged current.
    assert_eq!(days[first_current]["date"], "2026-01-01");
    // It carries Previous-Week traffic too, so it also gets a "previous" row —
    // the one date that appears twice, previous half first.
    assert_eq!(days[first_current - 1]["date"], "2026-01-01");
    assert_eq!(days[first_current - 1]["week"], "previous");
    let prev = window(&r, "previous");
    assert_eq!(prev.iter().map(|d| n(&d["fresh"])).sum::<i64>(), PREV_FRESH);
    assert_eq!(window(&r, "current").len(), 8);
}
