//! The shared store: append-only JSONL, one file per **Session**, under
//! `~/.cca/sessions/`. The **Proxy** appends `req` records; `claude-dash` reads
//! them. A pure reader, file-watched for liveness.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::record::{Record, ReqRecord};

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

/// Pick the newest `req` record across many records — the freshest **Budget**
/// reading wins, since **Budget** is account-wide and any **Session**'s latest
/// reading reflects the whole subscription.
///
/// Pure over an iterator of records so it's testable without touching the
/// filesystem.
pub fn newest_req<'a, I>(records: I) -> Option<&'a ReqRecord>
where
    I: IntoIterator<Item = &'a Record>,
{
    records
        .into_iter()
        .filter_map(Record::as_req)
        .max_by_key(|req| req.ts)
}

/// Glob `~/.cca/sessions/*.jsonl`, read every file, and return the newest `req`
/// record across all of them (account-wide newest **Budget**).
pub fn newest_req_in_dir(dir: &Path) -> Option<ReqRecord> {
    let pattern = dir.join("*.jsonl");
    let pattern = pattern.to_string_lossy();

    let all: Vec<Record> = glob::glob(&pattern)
        .ok()?
        .filter_map(|p| p.ok())
        .flat_map(|p| read_records(&p))
        .collect();

    newest_req(&all).cloned()
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
        ))
    }

    #[test]
    fn newest_req_picks_highest_timestamp() {
        let records = vec![req(100, 0.1), req(300, 0.3), req(200, 0.2)];
        let newest = newest_req(&records).unwrap();
        assert_eq!(newest.ts, 300);
        assert_eq!(newest.budget.b5_util, 0.3);
    }

    #[test]
    fn newest_req_is_none_when_empty() {
        let records: Vec<Record> = vec![];
        assert!(newest_req(&records).is_none());
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

        let newest = newest_req_in_dir(dir.path()).expect("a req exists");
        assert_eq!(newest.ts, 400);
        assert_eq!(newest.budget.b5_util, 0.4);
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
