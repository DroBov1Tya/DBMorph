use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread::JoinHandle;

use anyhow::{Result, anyhow, bail};

use encoding_rs::Encoding;

use crate::app::transform::encoding;
use crate::app::utils::ui;
use crate::args::AppArgs;
use crate::config;

// Row hygiene shared by every reader: pads short rows, drops runaway-wide
// ones, optionally truncates fields, and counts skips for the exit code.
#[derive(Clone)]
pub struct RowGuard {
    expected: usize,
    max_field: usize,
    strict: bool,
    trim: Arc<[char]>,
    mojibake: Option<&'static Encoding>,
    unmojibaked: Arc<AtomicU64>,
    skipped: Arc<AtomicU64>,
    warned: Arc<AtomicU64>,
}

impl RowGuard {
    pub fn new(expected: usize, max_field: usize, strict: bool, trim: &Option<String>) -> Self {
        let trim: Arc<[char]> = trim
            .as_deref()
            .map(|s| s.chars().collect::<Vec<char>>())
            .unwrap_or_default()
            .into();
        Self {
            expected,
            max_field,
            strict,
            trim,
            mojibake: None,
            unmojibaked: Arc::new(AtomicU64::new(0)),
            skipped: Arc::new(AtomicU64::new(0)),
            warned: Arc::new(AtomicU64::new(0)),
        }
    }

    // The guard as configured by the command line.
    pub fn for_args(expected: usize, args: &AppArgs) -> Self {
        let mut guard = Self::new(expected, args.max_field, args.strict, &args.trim);
        guard.mojibake = args.fix_mojibake.as_deref().and_then(encoding::by_label);
        guard
    }

    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    // Normalizes one record: Ok(Some) to emit, Ok(None) to skip, Err in strict mode.
    fn check(&self, index: u64, mut row: Vec<String>) -> Result<Option<Vec<String>>> {
        let got = row.len();

        if got != self.expected {
            let reason = if got > self.expected {
                "too many fields (lost delimiter / unquoted separator?)"
            } else {
                "too few fields"
            };
            if self.strict {
                bail!(
                    "row {}: {reason}: got {got}, expected {}",
                    index + 1,
                    self.expected
                );
            }
            self.record_skip(index, got, reason);

            if got
                > self
                    .expected
                    .saturating_mul(config::SQL_TUPLE_FIELD_FACTOR)
                    .max(config::SQL_TUPLE_FIELD_MIN)
            {
                return Ok(None);
            }
            if got > self.expected {
                row.truncate(self.expected);
            } else {
                row.resize(self.expected, String::new());
            }
        }

        if let Some(wrong) = self.mojibake {
            for cell in row.iter_mut() {
                if let Some(fixed) = encoding::unmojibake(cell, wrong) {
                    *cell = fixed;
                    self.unmojibaked.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        if !self.trim.is_empty() {
            let pat: &[char] = &self.trim;
            for cell in row.iter_mut() {
                let trimmed = cell.trim_matches(pat);
                if trimmed.len() != cell.len() {
                    *cell = trimmed.to_string();
                }
            }
        }

        if self.max_field > 0 {
            for cell in row.iter_mut() {
                truncate_in_place(cell, self.max_field);
            }
        }

        Ok(Some(row))
    }

    fn record_skip(&self, index: u64, got: usize, reason: &str) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
        let warned = self.warned.fetch_add(1, Ordering::Relaxed);
        if warned < config::SKIP_WARN_LIMIT {
            ui::warn(&format!(
                "row {}: {reason} ({got} fields) - realigned",
                index + 1
            ));
        } else if warned == config::SKIP_WARN_LIMIT {
            ui::warn("further malformed-row warnings suppressed");
        }
    }

    // Prints a closing summary and returns how many rows or cells were
    // changed, for the exit code.
    pub fn finish(&self) -> u64 {
        let n = self.skipped();
        if n > 0 {
            ui::warn(&format!("{n} malformed row(s) realigned or dropped"));
        }
        let fixed = self.unmojibaked.load(Ordering::Relaxed);
        if fixed > 0 {
            ui::warn(&format!("repaired mojibake in {fixed} cell(s)"));
        }
        n + fixed
    }
}

// Streams a raw row iterator through the guard.
pub fn guarded<I>(rows: I, guard: RowGuard) -> impl Iterator<Item = Result<Vec<String>>>
where
    I: Iterator<Item = Result<Vec<String>>>,
{
    rows.enumerate().filter_map(move |(i, res)| {
        let i = i as u64;
        match res {
            Ok(row) => match guard.check(i, row) {
                Ok(Some(r)) => Some(Ok(r)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            },
            Err(e) => Some(Err(e)),
        }
    })
}

// Runs a row source on its own thread so reading, decoding and the row guard
// overlap with the writer. Rows travel in bounded batches (by rows and by
// bytes) and an error ends the stream. The first batch is preview-sized so the
// preview appears at once. Dropping the returned iterator stops the reader
// thread at its next batch.
pub fn pipelined<I>(rows: I) -> Result<impl Iterator<Item = Result<Vec<String>>>>
where
    I: Iterator<Item = Result<Vec<String>>> + Send + 'static,
{
    let (tx, rx) = sync_channel::<Vec<Result<Vec<String>>>>(config::PIPELINE_DEPTH);

    let handle = std::thread::Builder::new()
        .name("dbmorph-reader".to_string())
        .spawn(move || {
            let mut batch = Vec::with_capacity(config::PIPELINE_BATCH_ROWS);
            let mut batch_rows = config::PREVIEW_ROWS;
            let mut batch_bytes = 0usize;
            for row in rows {
                let failed = row.is_err();
                if let Ok(cells) = &row {
                    batch_bytes += cells.iter().map(String::len).sum::<usize>();
                }
                batch.push(row);
                if failed
                    || batch.len() >= batch_rows
                    || batch_bytes >= config::PIPELINE_BATCH_BYTES
                {
                    let full = std::mem::replace(
                        &mut batch,
                        Vec::with_capacity(config::PIPELINE_BATCH_ROWS),
                    );
                    if tx.send(full).is_err() || failed {
                        return;
                    }
                    batch_rows = config::PIPELINE_BATCH_ROWS;
                    batch_bytes = 0;
                }
            }
            if !batch.is_empty() {
                // The writer may already be gone; nothing is lost then.
                let _ = tx.send(batch);
            }
        })?;

    Ok(Pipeline {
        rx,
        current: Vec::new().into_iter(),
        handle: Some(handle),
    })
}

struct Pipeline {
    rx: Receiver<Vec<Result<Vec<String>>>>,
    current: std::vec::IntoIter<Result<Vec<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl Iterator for Pipeline {
    type Item = Result<Vec<String>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(row) = self.current.next() {
                return Some(row);
            }
            match self.rx.recv() {
                Ok(batch) => self.current = batch.into_iter(),
                // The channel closes when the reader is done; a panic there
                // must surface as an error, not as a silently short output.
                Err(_) => {
                    let handle = self.handle.take()?;
                    return handle
                        .join()
                        .err()
                        .map(|_| Err(anyhow!("reader thread panicked")));
                }
            }
        }
    }
}

fn truncate_in_place(s: &mut String, max_chars: usize) {
    if let Some((byte_idx, _)) = s.char_indices().nth(max_chars) {
        s.truncate(byte_idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(n: usize) -> impl Iterator<Item = Result<Vec<String>>> + Send + 'static {
        (0..n).map(|i| Ok(vec![i.to_string()]))
    }

    #[test]
    fn pipeline_keeps_every_row_in_order_across_batches() {
        let n = config::PIPELINE_BATCH_ROWS * 3 + 7;
        let out: Vec<Vec<String>> = pipelined(numbered(n))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(out.len(), n);
        assert!(out.iter().enumerate().all(|(i, r)| r[0] == i.to_string()));
    }

    #[test]
    fn pipeline_handles_an_empty_source() {
        assert_eq!(pipelined(numbered(0)).unwrap().count(), 0);
    }

    #[test]
    fn pipeline_stops_after_the_first_error() {
        let rows = (0..10).map(|i| {
            if i == 3 {
                Err(anyhow!("bad row"))
            } else {
                Ok(vec![i.to_string()])
            }
        });
        let out: Vec<Result<Vec<String>>> = pipelined(rows).unwrap().collect();
        assert_eq!(out.len(), 4);
        assert!(out[3].is_err());
    }

    #[test]
    fn pipeline_reports_a_reader_panic_as_an_error() {
        let rows = (0..5).map(|i| {
            if i == 2 {
                panic!("reader bug");
            }
            Ok(vec![i.to_string()])
        });
        let out: Vec<Result<Vec<String>>> = pipelined(rows).unwrap().collect();
        assert!(
            out.last().unwrap().is_err(),
            "panic must not look like a clean end"
        );
    }

    // Endless source that flags when the reader thread drops it.
    struct Endless(Arc<std::sync::atomic::AtomicBool>);

    impl Iterator for Endless {
        type Item = Result<Vec<String>>;
        fn next(&mut self) -> Option<Self::Item> {
            Some(Ok(vec!["x".to_string()]))
        }
    }

    impl Drop for Endless {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn dropping_the_consumer_stops_the_reader() {
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut it = pipelined(Endless(stopped.clone())).unwrap();
        assert!(it.next().is_some());
        drop(it);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !stopped.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "reader thread kept running"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn wide_rows_still_arrive_in_order() {
        let wide = "x".repeat(config::PIPELINE_BATCH_BYTES / 3);
        let rows = (0..10).map(move |i| Ok(vec![i.to_string(), wide.clone()]));
        let out: Vec<Vec<String>> = pipelined(rows).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(out.len(), 10);
        assert!(out.iter().enumerate().all(|(i, r)| r[0] == i.to_string()));
    }

    #[test]
    fn guard_pads_truncates_and_trims() {
        let guard = RowGuard::new(2, 3, false, &Some("\"".to_string()));
        let rows = vec![
            Ok(vec!["\"abcdef\"".to_string()]),
            Ok(vec!["a".into(), "b".into(), "c".into()]),
        ];
        let out: Vec<Vec<String>> = guarded(rows.into_iter(), guard.clone())
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(out, vec![vec!["abc", ""], vec!["a", "b"]]);
        assert_eq!(guard.skipped(), 2);
    }
}
