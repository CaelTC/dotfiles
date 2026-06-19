//! The shared store: append-only JSONL, one file per **Session**, under
//! `~/.cca/sessions/`. The **Proxy** appends `req` records; `claude-dash` reads
//! them. A pure reader, file-watched for liveness.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::record::{EndRecord, Record, ReqRecord, StartRecord};

/// The store directory `~/.cca/sessions`.
pub fn sessions_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".cca").join("sessions"))
}

/// The JSONL file for one **Session** id: `~/.cca/sessions/<id>.jsonl`.
pub fn session_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.jsonl"))
}

/// Append one record to a **Session**'s JSONL file as a single line, creating
/// the file (and parent directories) if needed. Append-only and line-oriented,
/// so concurrent **Proxy** writers don't contend.
pub fn append_record(path: &Path, record: &Record) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating store dir {}", parent.display()))?;
    }
    let mut line = serde_json::to_string(record).context("serializing record")?;
    line.push('\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening session file {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("appending to {}", path.display()))?;
    Ok(())
}

/// Read every parseable [`Record`] from a single JSONL file. Lines that fail to
/// parse are skipped so a partially-written tail line can't break a read.
pub fn read_records(path: &Path) -> Vec<Record> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Record>(l).ok())
        .collect()
}

/// List the session files in the store: `~/.cca/sessions/*.jsonl`. Both readers
/// start here; they differ only in how they select across the files.
fn session_files(dir: &Path) -> Vec<PathBuf> {
    let pattern = dir.join("*.jsonl");
    match glob::glob(&pattern.to_string_lossy()) {
        Ok(paths) => paths.filter_map(|p| p.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// One **Session**'s view of the store: its id (the JSONL file stem), its `start`
/// record fields when present, and its `req` records in append order.
///
/// This is the store's single per-**Session** grouping primitive. Every reader is
/// a thin selector over a `Vec<SessionView>`: account-wide **Budget** flattens all
/// sessions' `req`s and picks the newest; the **Active Session** panels render one
/// per view. A later slice's History view will be a third selector over the same
/// shape.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    /// The **Session** id — the `<id>.jsonl` file stem.
    pub id: String,
    /// The **Session**'s `start` record fields (project, cwd, pid, start ts) when
    /// it wrote one; `None` for a session whose file carries only `req`s.
    pub start: Option<StartRecord>,
    /// The **Session**'s `req` records, in append (file) order, so a caller can
    /// window them by `ts` for the rolling **Throughput** rate.
    pub reqs: Vec<ReqRecord>,
    /// The **Session**'s `end` record when `cca` wrote one (`claude` exited
    /// normally); `None` while the **Session** is still running or `cca` was
    /// killed without writing it. The lifecycle classifier reads this — plus
    /// pid-liveness for the `None` case — to split active from **Session
    /// History**.
    pub end: Option<EndRecord>,
}

/// Group `(session_id, records)` streams into per-**Session** [`SessionView`]s.
///
/// The grouping primitive the store is built around: each input pairs a
/// **Session** id (the file stem) with that file's records in append order. The
/// `start` record (if any) supplies the view's identity fields; the `req`s are
/// kept in order. Pure over its inputs — no filesystem — so it's unit-testable
/// directly.
pub fn group_sessions<I, S>(sessions: I) -> Vec<SessionView>
where
    I: IntoIterator<Item = (S, Vec<Record>)>,
    S: Into<String>,
{
    sessions
        .into_iter()
        .map(|(id, records)| {
            let start = records.iter().find_map(|r| r.as_start().cloned());
            let reqs = records.iter().filter_map(|r| r.as_req().cloned()).collect();
            let end = records.iter().find_map(|r| r.as_end().cloned());
            SessionView {
                id: id.into(),
                start,
                reqs,
                end,
            }
        })
        .collect()
}

/// Glob `~/.cca/sessions/*.jsonl` and read each file into a [`SessionView`] keyed
/// by its file stem (the **Session** id). The thin dir-reading wrapper over the
/// pure [`group_sessions`] primitive.
pub fn session_views_in_dir(dir: &Path) -> Vec<SessionView> {
    let sessions = session_files(dir).into_iter().map(|path| {
        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        (id, read_records(&path))
    });
    group_sessions(sessions)
}

/// Pick the newest `req` by `ts` from an iterator of [`ReqRecord`]s — the
/// freshest **Budget** reading wins, since **Budget** is account-wide and any
/// **Session**'s latest reading reflects the whole subscription.
///
/// Pure over its iterator so it's testable without touching the filesystem; both
/// the in-memory and the [`SessionView`] selectors funnel through it.
pub fn newest_req<'a, I>(reqs: I) -> Option<&'a ReqRecord>
where
    I: IntoIterator<Item = &'a ReqRecord>,
{
    reqs.into_iter().max_by_key(|req| req.ts)
}

/// The account-wide **Budget** selector: flatten every **Session**'s `req`s and
/// take the newest by `ts`. A thin selector over [`SessionView`]s — **Budget** is
/// account-wide, so the freshest reading across all sessions wins.
pub fn newest_req_in_views(views: &[SessionView]) -> Option<&ReqRecord> {
    newest_req(views.iter().flat_map(|v| v.reqs.iter()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::Budget;

    fn req(ts: i64, b5: f64) -> Record {
        Record::Req(ReqRecord::from_budget(
            &Budget {
                b5_util: b5,
                b5_reset: 1_750_000_000,
                b7_util: 0.1,
                b7_reset: 1_750_500_000,
                rep: "five_hour".to_string(),
                status: "allowed".to_string(),
            },
            ts,
            None,
        ))
    }

    /// A bare [`ReqRecord`] (not wrapped in [`Record`]) for the pure-iterator
    /// `newest_req` tests.
    fn req_record(ts: i64, b5: f64) -> ReqRecord {
        req(ts, b5).as_req().unwrap().clone()
    }

    #[test]
    fn newest_req_picks_highest_timestamp() {
        let reqs = vec![req_record(100, 0.1), req_record(300, 0.3), req_record(200, 0.2)];
        let newest = newest_req(&reqs).unwrap();
        assert_eq!(newest.ts, 300);
        assert_eq!(newest.budget.b5_util, 0.3);
    }

    #[test]
    fn newest_req_is_none_when_empty() {
        let reqs: Vec<ReqRecord> = vec![];
        assert!(newest_req(&reqs).is_none());
    }

    #[test]
    fn append_then_newest_across_files_picks_freshest() {
        let dir = tempfile::tempdir().unwrap();

        // Session A: an older then a middling reading.
        let a = session_path(dir.path(), "aaaa");
        append_record(&a, &req(100, 0.1)).unwrap();
        append_record(&a, &req(250, 0.25)).unwrap();

        // Session B: the freshest reading account-wide.
        let b = session_path(dir.path(), "bbbb");
        append_record(&b, &req(400, 0.4)).unwrap();

        // Budget = newest req over the per-Session views (account-wide selector).
        let views = session_views_in_dir(dir.path());
        let newest = newest_req_in_views(&views).expect("a req exists");
        assert_eq!(newest.ts, 400);
        assert_eq!(newest.budget.b5_util, 0.4);
    }

    fn start(id: &str, project: &str, pid: i32) -> Record {
        Record::Start(StartRecord {
            id: id.to_string(),
            ts: 1,
            project: project.to_string(),
            cwd: format!("/work/{project}"),
            pid,
        })
    }

    #[test]
    fn group_sessions_carries_start_fields_and_ordered_reqs() {
        let views = group_sessions(vec![(
            "aaaa",
            vec![start("aaaa", "proj-a", 11), req(100, 0.1), req(200, 0.2)],
        )]);
        assert_eq!(views.len(), 1);
        let v = &views[0];
        assert_eq!(v.id, "aaaa");
        let s = v.start.as_ref().expect("start present");
        assert_eq!(s.project, "proj-a");
        assert_eq!(s.pid, 11);
        // reqs preserved in append order.
        assert_eq!(v.reqs.len(), 2);
        assert_eq!(v.reqs[0].ts, 100);
        assert_eq!(v.reqs[1].ts, 200);
    }

    fn end(id: &str, ts: i64) -> Record {
        Record::End(crate::record::EndRecord {
            id: id.to_string(),
            ts,
        })
    }

    #[test]
    fn group_sessions_carries_end_record() {
        let views = group_sessions(vec![(
            "aaaa",
            vec![
                start("aaaa", "proj-a", 11),
                req(100, 0.1),
                req(200, 0.2),
                end("aaaa", 900),
            ],
        )]);
        assert_eq!(views.len(), 1);
        let v = &views[0];
        // start + end + reqs all round-trip into the view.
        assert_eq!(v.start.as_ref().unwrap().project, "proj-a");
        assert_eq!(v.reqs.len(), 2);
        let e = v.end.as_ref().expect("end present");
        assert_eq!(e.id, "aaaa");
        assert_eq!(e.ts, 900);
    }

    #[test]
    fn group_sessions_end_is_none_when_no_end_record() {
        let views = group_sessions(vec![("aaaa", vec![start("aaaa", "proj-a", 11), req(100, 0.1)])]);
        assert!(views[0].end.is_none());
    }

    #[test]
    fn group_sessions_start_is_none_when_only_reqs() {
        let views = group_sessions(vec![("bbbb", vec![req(300, 0.3)])]);
        assert_eq!(views.len(), 1);
        assert!(views[0].start.is_none());
        assert_eq!(views[0].reqs.len(), 1);
    }

    #[test]
    fn multiple_sessions_yield_multiple_views() {
        let dir = tempfile::tempdir().unwrap();

        let a = session_path(dir.path(), "aaaa");
        append_record(&a, &start("aaaa", "proj-a", 11)).unwrap();
        append_record(&a, &req(100, 0.1)).unwrap();

        let b = session_path(dir.path(), "bbbb");
        append_record(&b, &start("bbbb", "proj-b", 22)).unwrap();
        append_record(&b, &req(400, 0.4)).unwrap();

        let mut views = session_views_in_dir(dir.path());
        views.sort_by(|x, y| x.id.cmp(&y.id));
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].id, "aaaa");
        assert_eq!(views[0].start.as_ref().unwrap().project, "proj-a");
        assert_eq!(views[1].id, "bbbb");
        assert_eq!(views[1].start.as_ref().unwrap().project, "proj-b");
    }

    #[test]
    fn newest_req_in_views_picks_freshest_across_sessions() {
        let views = group_sessions(vec![
            ("aaaa", vec![req(100, 0.1), req(250, 0.25)]),
            ("bbbb", vec![req(400, 0.4)]),
        ]);
        let newest = newest_req_in_views(&views).expect("a req exists");
        assert_eq!(newest.ts, 400);
        assert_eq!(newest.budget.b5_util, 0.4);
    }

    #[test]
    fn session_views_is_empty_when_no_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(session_views_in_dir(dir.path()).is_empty());
    }

    #[test]
    fn read_records_skips_unparseable_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = session_path(dir.path(), "cccc");
        append_record(&path, &req(10, 0.1)).unwrap();
        // Simulate a partially-written / garbage tail line.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{not valid json").unwrap();

        let recs = read_records(&path);
        assert_eq!(recs.len(), 1);
    }
}
