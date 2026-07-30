//! The five groupings. Each independently accounts for 100% of the Week, and
//! carries `prevFresh` — the same attribution over the Previous Week — so a
//! colleague can see what grew, shrank, or disappeared.

use crate::parse::{Request, CATEGORIES};
use chrono::{DateTime, Duration, Local, NaiveDate, SecondsFormat, TimeZone, Utc};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const NO_SKILL: &str = "(no skill)";

/// Everything read out of one session's transcripts: the main conversation plus
/// the subagents it spawned. Holds both windows' requests; the session *row* is
/// current-Week only.
#[derive(Default)]
pub struct Session {
    pub project: String,
    pub first_prompt: String,
    /// Main-conversation requests, in transcript line order.
    pub main: Vec<Request>,
    pub agents: BTreeMap<String, Agent>,
    /// `Agent` tool_use id -> `message.id` of the spawning request.
    pub spawns: HashMap<String, String>,
}

#[derive(Default)]
pub struct Agent {
    pub model: String,
    pub requests: Vec<Request>,
    /// `toolUseId` from the sidecar `agent-*.meta.json`, when present.
    pub tool_use_id: Option<String>,
}

#[derive(Default)]
struct Tally {
    fresh: i64,
    cache_read: i64,
    requests: i64,
}

impl Tally {
    fn add(&mut self, r: &Request) {
        self.fresh += r.fresh;
        self.cache_read += r.cache_read;
        self.requests += 1;
    }
}

#[derive(Default)]
struct Skill {
    /// Current-Week runs only.
    runs: i64,
    main: i64,
    agents: i64,
    cache_read: i64,
    requests: i64,
    prev_fresh: i64,
}

pub fn report(
    sessions: BTreeMap<String, Session>,
    skipped: usize,
    now: DateTime<Utc>,
    week_start: DateTime<Utc>,
    prev_start: DateTime<Utc>,
) -> Value {
    let is_prev = |r: &Request| r.ts < week_start;
    let mut skills: BTreeMap<String, Skill> = BTreeMap::new();
    let mut session_rows: Vec<Value> = Vec::new();
    let mut all: Vec<&Request> = Vec::new();
    let mut all_prev: Vec<&Request> = Vec::new();

    for (id, s) in &sessions {
        let skill_of = skill_names(&s.main);
        count_runs(s.main.iter().filter(|r| !is_prev(r)), &mut skills);
        for r in &s.main {
            let name = skill_of.get(&r.id).cloned().unwrap_or_else(|| NO_SKILL.to_string());
            let e = skills.entry(name).or_default();
            if is_prev(r) {
                e.prev_fresh += r.fresh;
            } else {
                e.main += r.fresh;
                e.cache_read += r.cache_read;
                e.requests += 1;
            }
        }

        let mut agent_rows: Vec<Value> = Vec::new();
        for (aid, a) in &s.agents {
            if a.requests.is_empty() {
                continue;
            }
            let name = agent_skill(a, s, &skill_of);
            let mut at = Tally::default();
            for r in &a.requests {
                let e = skills.entry(name.clone()).or_default();
                if is_prev(r) {
                    e.prev_fresh += r.fresh;
                } else {
                    e.agents += r.fresh;
                    e.cache_read += r.cache_read;
                    e.requests += 1;
                    at.add(r);
                }
            }
            if at.requests == 0 {
                continue; // the agent ran in the Previous Week
            }
            agent_rows.push(json!({
                "id": short(aid),
                "model": a.model,
                "fresh": at.fresh,
                "cacheRead": at.cache_read,
                "requests": at.requests,
            }));
        }
        agent_rows.sort_by_key(|a| -a["fresh"].as_i64().unwrap_or(0));

        // Session totals include the agents it spawned.
        let mut mine: Vec<&Request> = Vec::new();
        for r in s.main.iter().chain(s.agents.values().flat_map(|a| a.requests.iter())) {
            if is_prev(r) {
                all_prev.push(r);
            } else {
                mine.push(r);
            }
        }
        all.extend(&mine);
        if mine.is_empty() {
            continue; // nothing this Week — the sessions list is current-only
        }
        let mut sess = Tally::default();
        let mut cats: HashMap<&str, i64> = HashMap::new();
        for r in &mine {
            sess.add(r);
            *cats.entry(r.category).or_default() += r.fresh;
        }
        session_rows.push(json!({
            "id": short(id),
            "project": s.project,
            "firstPrompt": s.first_prompt,
            "start": iso(mine.iter().map(|r| r.ts).min().unwrap()),
            "end": iso(mine.iter().map(|r| r.ts).max().unwrap()),
            "fresh": sess.fresh,
            "cacheRead": sess.cache_read,
            "requests": sess.requests,
            "byCategory": cat_map(&cats),
            "agents": agent_rows,
        }));
    }
    session_rows.sort_by_key(|s| -s["fresh"].as_i64().unwrap_or(0));

    let boundary = week_start.with_timezone(&Local).date_naive();
    let mut totals = Tally::default();
    let mut prev_fresh = 0i64;
    let mut by_category: HashMap<&str, Tally> = HashMap::new();
    let mut by_server: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_model: BTreeMap<String, Tally> = BTreeMap::new();
    let mut prev_category: BTreeMap<String, i64> = BTreeMap::new();
    let mut prev_server: BTreeMap<String, i64> = BTreeMap::new();
    let mut prev_model: BTreeMap<String, i64> = BTreeMap::new();
    // Day buckets are keyed by window as well as date: the boundary date holds
    // the tail of the Previous Week and the head of this one, and blending them
    // would break one grouping's sum or lose the difference.
    let mut by_day: BTreeMap<(NaiveDate, bool), HashMap<&str, i64>> = BTreeMap::new();
    let mut day_tally: BTreeMap<(NaiveDate, bool), Tally> = BTreeMap::new();
    let mut bucket = |r: &Request, is_prev: bool| {
        let key = (r.ts.with_timezone(&Local).date_naive(), is_prev);
        *by_day.entry(key).or_default().entry(r.category).or_default() += r.fresh;
        day_tally.entry(key).or_default().add(r);
    };

    for r in &all {
        totals.add(r);
        by_category.entry(r.category).or_default().add(r);
        by_model.entry(r.model.clone()).or_default().add(r);
        if r.category == "MCP" {
            by_server.entry(server_of(r)).or_default().add(r);
        }
        bucket(r, false);
    }
    for r in &all_prev {
        prev_fresh += r.fresh;
        *prev_category.entry(r.category.to_string()).or_default() += r.fresh;
        *prev_model.entry(r.model.clone()).or_default() += r.fresh;
        if r.category == "MCP" {
            *prev_server.entry(server_of(r)).or_default() += r.fresh;
        }
        bucket(r, true);
    }

    json!({
        "generatedAt": iso(now),
        "weekStart": iso(week_start),
        "weekEnd": iso(now),
        "prevWeekStart": iso(prev_start),
        "totals": {
            "fresh": totals.fresh,
            "cacheRead": totals.cache_read,
            "requests": totals.requests,
            "sessions": session_rows.len(),
            "prevFresh": prev_fresh,
        },
        "skippedRecords": skipped,
        "categories": CATEGORIES.iter().map(|c| {
            let mut row = tally_row(c, by_category.get(c), prev_category.get(*c).copied());
            if *c == "MCP" {
                row["servers"] = Value::Array(named_rows(&by_server, &prev_server));
            }
            row
        }).collect::<Vec<_>>(),
        "models": named_rows(&by_model, &prev_model),
        "days": days(prev_start, boundary, now, &by_day, &day_tally),
        "sessions": session_rows,
        "skills": skill_rows(skills),
    })
}

/// The skill active for each request, straight off `attributionSkill`.
fn skill_names(main: &[Request]) -> HashMap<String, String> {
    main.iter()
        .filter_map(|r| r.skill.as_ref().map(|s| (r.id.clone(), s.clone())))
        .collect()
}

/// Consecutive same-skill requests form one run. Current Week only — the
/// Previous Week contributes tokens, not run counts.
fn count_runs<'a>(
    main: impl Iterator<Item = &'a Request>,
    skills: &mut BTreeMap<String, Skill>,
) {
    let mut previous: Option<String> = None;
    for r in main {
        if let Some(name) = &r.skill {
            if previous.as_ref() != Some(name) {
                skills.entry(name.clone()).or_default().runs += 1;
            }
        }
        previous = r.skill.clone();
    }
}

/// An agent joins the skill run active when it was spawned. Authoritative link:
/// the sidecar meta's `toolUseId` -> the parent's `Agent` tool_use -> that
/// request's skill. Falls back to timestamp containment.
fn agent_skill(a: &Agent, s: &Session, skill_of: &HashMap<String, String>) -> String {
    let direct = a
        .tool_use_id
        .as_ref()
        .and_then(|t| s.spawns.get(t))
        .and_then(|mid| skill_of.get(mid));
    if let Some(name) = direct {
        return name.clone();
    }
    let spawn = a.requests.iter().map(|r| r.ts).min();
    let containing = spawn.and_then(|ts| {
        s.main
            .iter()
            .filter(|r| r.ts <= ts)
            .max_by_key(|r| r.ts)
            .and_then(|r| skill_of.get(&r.id))
    });
    containing.cloned().unwrap_or_else(|| NO_SKILL.to_string())
}

/// Every local calendar day from the Previous Week's start to now, oldest
/// first. `week` says which window a day belongs to; the boundary day counts as
/// current. Only the oldest can be partial.
///
/// The boundary date is the one day the two windows share. It gets a second,
/// earlier row tagged `"previous"` when it actually carries Previous-Week
/// traffic — without it, either the current rows would over-count or that
/// traffic would vanish from the grouping entirely.
fn days(
    prev_start: DateTime<Utc>,
    boundary: NaiveDate,
    now: DateTime<Utc>,
    by_day: &BTreeMap<(NaiveDate, bool), HashMap<&str, i64>>,
    tally: &BTreeMap<(NaiveDate, bool), Tally>,
) -> Vec<Value> {
    let first = prev_start.with_timezone(&Local).date_naive();
    let last = now.with_timezone(&Local).date_naive();
    let partial = Local
        .from_local_datetime(&first.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .map(|midnight| midnight.with_timezone(&Utc) < prev_start)
        .unwrap_or(true);
    let mut out = Vec::new();
    let mut d = first;
    while d <= last {
        for is_prev in [true, false] {
            let key = (d, is_prev);
            // One row per date, except the shared boundary date.
            let belongs = if is_prev { d < boundary } else { d >= boundary };
            if !belongs && !(d == boundary && is_prev && tally.contains_key(&key)) {
                continue;
            }
            let t = tally.get(&key);
            out.push(json!({
                "date": d.format("%Y-%m-%d").to_string(),
                "week": if is_prev { "previous" } else { "current" },
                "partial": d == first && partial,
                "fresh": t.map_or(0, |t| t.fresh),
                "cacheRead": t.map_or(0, |t| t.cache_read),
                "byCategory": cat_map(&by_day.get(&key).cloned().unwrap_or_default()),
            }));
        }
        d = d.succ_opt().unwrap();
    }
    out
}

/// Heaviest first, `(no skill)` last. A skill that ran only in the Previous
/// Week keeps a row, with zeroed current fields, so its absence is visible.
fn skill_rows(skills: BTreeMap<String, Skill>) -> Vec<Value> {
    let mut rows: Vec<Value> = skills
        .into_iter()
        .filter(|(_, s)| s.requests > 0 || s.prev_fresh > 0)
        .map(|(name, s)| {
            let fresh = s.main + s.agents;
            json!({
                "name": name,
                "runs": s.runs,
                "fresh": fresh,
                "cacheRead": s.cache_read,
                "requests": s.requests,
                "avgFreshPerRun": if s.runs > 0 { fresh / s.runs } else { 0 },
                "prevFresh": s.prev_fresh,
                "composition": { "main": s.main, "agents": s.agents },
            })
        })
        .collect();
    rows.sort_by_key(|r| (r["name"].as_str() == Some(NO_SKILL), -r["fresh"].as_i64().unwrap_or(0)));
    rows
}

fn tally_row(name: &str, t: Option<&Tally>, prev: Option<i64>) -> Value {
    json!({
        "name": name,
        "fresh": t.map_or(0, |t| t.fresh),
        "cacheRead": t.map_or(0, |t| t.cache_read),
        "requests": t.map_or(0, |t| t.requests),
        "prevFresh": prev.unwrap_or(0),
    })
}

/// Named tallies as rows, heaviest first. Names seen only in the Previous Week
/// still get a row, so a dropped model or server does not just vanish.
fn named_rows(cur: &BTreeMap<String, Tally>, prev: &BTreeMap<String, i64>) -> Vec<Value> {
    let names: BTreeSet<&String> = cur.keys().chain(prev.keys()).collect();
    let mut rows: Vec<Value> = names
        .into_iter()
        .map(|name| tally_row(name, cur.get(name), prev.get(name).copied()))
        .collect();
    rows.sort_by_key(|r| -r["fresh"].as_i64().unwrap_or(0));
    rows
}

fn server_of(r: &Request) -> String {
    r.mcp_server.clone().unwrap_or_else(|| "(unknown)".to_string())
}

/// All six Categories present, so the template never has to guard for holes.
fn cat_map(src: &HashMap<&str, i64>) -> Value {
    let mut m = Map::new();
    for c in CATEGORIES {
        m.insert(c.to_string(), json!(src.get(c).copied().unwrap_or(0)));
    }
    Value::Object(m)
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn short(id: &str) -> String {
    id.trim_start_matches("agent-").chars().take(8).collect()
}

pub fn week_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::days(7)
}

/// The Previous Week runs from now−14d to `week_start` — anchored to today,
/// never a calendar week.
pub fn prev_week_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::days(14)
}
