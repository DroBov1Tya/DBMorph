use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, bail};

use crate::app::utils::ui;
use crate::config;

/// Row hygiene shared by every reader: pads short rows, drops runaway-wide
/// ones, optionally truncates fields, and counts skips for the exit code.
#[derive(Clone)]
pub struct RowGuard {
    expected: usize,
    max_field: usize,
    strict: bool,
    skipped: Arc<AtomicU64>,
    warned: Arc<AtomicU64>,
}

impl RowGuard {
    pub fn new(expected: usize, max_field: usize, strict: bool) -> Self {
        Self {
            expected,
            max_field,
            strict,
            skipped: Arc::new(AtomicU64::new(0)),
            warned: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    /// Normalizes one record: `Ok(Some)` to emit, `Ok(None)` to skip, `Err` in strict mode.
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

    /// Prints a closing summary and returns the skip count.
    pub fn finish(&self) -> u64 {
        let n = self.skipped();
        if n > 0 {
            ui::warn(&format!("{n} malformed row(s) realigned or dropped"));
        }
        n
    }
}

/// Streams a raw row iterator through the guard.
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

fn truncate_in_place(s: &mut String, max_chars: usize) {
    if let Some((byte_idx, _)) = s.char_indices().nth(max_chars) {
        s.truncate(byte_idx);
    }
}
