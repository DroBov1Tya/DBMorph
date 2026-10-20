use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;

use crate::app::transform::infer::{self, ColType};
use crate::config;

// Row-level repairs for delimited text, applied to parsed records before the
// row guard. Each one fixes one recognisable kind of damage, leaves every
// other row untouched, and counts what it changed.

#[derive(Clone, Default)]
pub struct RepairCounters {
    pub trailing: Arc<AtomicU64>,
    pub joined: Arc<AtomicU64>,
    pub realigned: Arc<AtomicU64>,
}

impl RepairCounters {
    pub fn joined(&self) -> u64 {
        self.joined.load(Ordering::Relaxed)
    }

    pub fn realigned(&self) -> u64 {
        self.realigned.load(Ordering::Relaxed)
    }

    pub fn trailing(&self) -> u64 {
        self.trailing.load(Ordering::Relaxed)
    }
}

type Row = Result<Vec<String>>;

// A delimiter at the end of every line yields one extra, empty field.
pub fn drop_trailing_empty<I>(
    rows: I,
    width: usize,
    dropped: Arc<AtomicU64>,
) -> impl Iterator<Item = Row>
where
    I: Iterator<Item = Row>,
{
    rows.map(move |row| {
        row.map(|mut cells| {
            if cells.len() == width + 1 && cells.last().is_some_and(String::is_empty) {
                cells.pop();
                dropped.fetch_add(1, Ordering::Relaxed);
            }
            cells
        })
    })
}

// Rejoins a row that a stray line break split into several short ones: a short
// row is merged with the following lines while that adds up to exactly the
// expected width; the break is kept inside the field as "\n". Anything that
// does not add up exactly is passed through unchanged.
pub fn join_broken_lines<I>(
    rows: I,
    width: usize,
    joined: Arc<AtomicU64>,
) -> impl Iterator<Item = Row>
where
    I: Iterator<Item = Row>,
{
    LineJoiner {
        rows,
        width,
        pending: VecDeque::new(),
        joined,
    }
}

struct LineJoiner<I> {
    rows: I,
    width: usize,
    pending: VecDeque<Row>,
    joined: Arc<AtomicU64>,
}

impl<I: Iterator<Item = Row>> LineJoiner<I> {
    fn pull(&mut self) -> Option<Row> {
        self.pending.pop_front().or_else(|| self.rows.next())
    }
}

impl<I: Iterator<Item = Row>> Iterator for LineJoiner<I> {
    type Item = Row;

    fn next(&mut self) -> Option<Row> {
        let row = match self.pull()? {
            Ok(row) if row.len() < self.width && !row.is_empty() => row,
            other => return Some(other),
        };

        let mut merged = row.clone();
        let mut taken: Vec<Vec<String>> = Vec::new();
        while merged.len() < self.width && taken.len() < config::MAX_JOIN_LINES {
            match self.pull() {
                Some(Ok(next)) if !next.is_empty() => {
                    append_line(&mut merged, &next);
                    taken.push(next);
                }
                Some(other) => {
                    self.pending.push_front(other);
                    break;
                }
                None => break,
            }
        }

        if merged.len() == self.width && !taken.is_empty() {
            self.joined.fetch_add(taken.len() as u64, Ordering::Relaxed);
            return Some(Ok(merged));
        }
        for line in taken.into_iter().rev() {
            self.pending.push_front(Ok(line));
        }
        Some(Ok(row))
    }
}

// The last field of the row and the first field of the next line were one
// field before the line break.
fn append_line(row: &mut Vec<String>, next: &[String]) {
    if let (Some(last), Some(first)) = (row.last_mut(), next.first()) {
        last.push('\n');
        last.push_str(first);
    }
    row.extend(next.iter().skip(1).cloned());
}

// Puts back fields that an unescaped delimiter inside a text value pushed one
// or more columns to the right. Column types are learned from the leading rows
// of the right width; a wide row is merged back only when exactly one text
// column can absorb the extra fields and every typed column then fits.
pub fn realign_shifted<I>(
    rows: I,
    width: usize,
    delimiter: u8,
    realigned: Arc<AtomicU64>,
) -> impl Iterator<Item = Row>
where
    I: Iterator<Item = Row>,
{
    Realigner {
        rows,
        width,
        delimiter: char::from(delimiter).to_string(),
        types: None,
        buffered: VecDeque::new(),
        realigned,
    }
}

struct Realigner<I> {
    rows: I,
    width: usize,
    delimiter: String,
    types: Option<Vec<ColType>>,
    buffered: VecDeque<Row>,
    realigned: Arc<AtomicU64>,
}

impl<I: Iterator<Item = Row>> Realigner<I> {
    // Reads ahead until enough clean rows were seen to infer column types.
    fn learn_types(&mut self) -> Vec<ColType> {
        let mut clean: Vec<Vec<String>> = Vec::new();
        while clean.len() < config::REALIGN_PROFILE_ROWS {
            let Some(row) = self.rows.next() else { break };
            if let Ok(cells) = &row
                && cells.len() == self.width
            {
                clean.push(cells.clone());
            }
            let failed = row.is_err();
            self.buffered.push_back(row);
            if failed {
                break;
            }
        }
        infer::infer_types(&clean, self.width)
    }

    fn realign(&self, cells: Vec<String>, types: &[ColType]) -> Vec<String> {
        let extra = cells.len().saturating_sub(self.width);
        if extra == 0 || extra > config::MAX_REALIGN_EXTRA {
            return cells;
        }

        let mut fits = (0..self.width)
            .filter(|&at| types[at] == ColType::Text)
            .map(|at| merge_at(&cells, at, extra, &self.delimiter))
            .filter(|candidate| fits_types(candidate, types));
        match (fits.next(), fits.next()) {
            (Some(only), None) => {
                self.realigned.fetch_add(1, Ordering::Relaxed);
                only
            }
            _ => cells,
        }
    }
}

impl<I: Iterator<Item = Row>> Iterator for Realigner<I> {
    type Item = Row;

    fn next(&mut self) -> Option<Row> {
        if self.types.is_none() {
            self.types = Some(self.learn_types());
        }
        let row = self.buffered.pop_front().or_else(|| self.rows.next())?;
        let types = self.types.as_deref().unwrap_or(&[]);
        Some(row.map(|cells| self.realign(cells, types)))
    }
}

// Joins fields at..=at+extra back into one, with the delimiter between them.
fn merge_at(cells: &[String], at: usize, extra: usize, delimiter: &str) -> Vec<String> {
    let mut merged = Vec::with_capacity(cells.len() - extra);
    merged.extend_from_slice(&cells[..at]);
    merged.push(cells[at..=at + extra].join(delimiter));
    merged.extend_from_slice(&cells[at + extra + 1..]);
    merged
}

fn fits_types(cells: &[String], types: &[ColType]) -> bool {
    cells
        .iter()
        .zip(types)
        .all(|(cell, ty)| infer::coerce(*ty, cell).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(raw: &[&[&str]]) -> Vec<Row> {
        raw.iter()
            .map(|r| Ok(r.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn collect(it: impl Iterator<Item = Row>) -> Vec<Vec<String>> {
        it.collect::<Result<_>>().unwrap()
    }

    #[test]
    fn trailing_empty_field_is_dropped_only_when_empty_and_extra() {
        let n = Arc::new(AtomicU64::new(0));
        let out = collect(drop_trailing_empty(
            rows(&[&["a", "b", ""], &["a", "b", "c"], &["a", ""]]).into_iter(),
            2,
            n.clone(),
        ));
        assert_eq!(
            out,
            vec![vec!["a", "b"], vec!["a", "b", "c"], vec!["a", ""]]
        );
        assert_eq!(n.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_row_split_by_a_line_break_is_rejoined() {
        let n = Arc::new(AtomicU64::new(0));
        let out = collect(join_broken_lines(
            rows(&[
                &["1", "first part"],
                &["second part", "x"],
                &["2", "ok", "y"],
            ])
            .into_iter(),
            3,
            n.clone(),
        ));
        assert_eq!(
            out,
            vec![
                vec!["1", "first part\nsecond part", "x"],
                vec!["2", "ok", "y"]
            ]
        );
        assert_eq!(n.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_row_split_over_several_lines_is_rejoined() {
        let n = Arc::new(AtomicU64::new(0));
        let out = collect(join_broken_lines(
            rows(&[&["1", "a"], &["b"], &["c", "z"]]).into_iter(),
            3,
            n.clone(),
        ));
        assert_eq!(out, vec![vec!["1", "a\nb\nc", "z"]]);
        assert_eq!(n.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn short_rows_that_do_not_add_up_are_left_alone() {
        let n = Arc::new(AtomicU64::new(0));
        let input = rows(&[&["1", "a"], &["2", "b", "c"], &["3"]]);
        let out = collect(join_broken_lines(input.into_iter(), 3, n.clone()));
        assert_eq!(out, vec![vec!["1", "a"], vec!["2", "b", "c"], vec!["3"]]);
        assert_eq!(n.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn joining_gives_up_after_the_line_limit() {
        let n = Arc::new(AtomicU64::new(0));
        let mut input = vec![Ok(vec!["start".to_string()])];
        input.extend((0..config::MAX_JOIN_LINES + 2).map(|_| Ok(vec!["x".to_string()])));
        let total = input.len();
        let out = collect(join_broken_lines(input.into_iter(), 50, n.clone()));
        assert_eq!(out.len(), total);
        assert_eq!(n.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn errors_pass_through_the_joiner_in_order() {
        let n = Arc::new(AtomicU64::new(0));
        let input = vec![
            Ok(vec!["1".to_string()]),
            Err(anyhow::anyhow!("bad")),
            Ok(vec!["2".to_string(), "b".to_string()]),
        ];
        let out: Vec<Row> = join_broken_lines(input.into_iter(), 2, n).collect();
        assert_eq!(out.len(), 3);
        assert!(out[1].is_err());
    }

    fn profile() -> Vec<&'static [&'static str]> {
        vec![
            &["1", "alice", "10.5"],
            &["2", "bob", "3"],
            &["3", "carol", "7.25"],
        ]
    }

    #[test]
    fn a_shifted_row_is_merged_back_into_the_only_text_column() {
        let n = Arc::new(AtomicU64::new(0));
        let mut raw = profile();
        raw.push(&["4", "smith", " john", "8"]);
        let out = collect(realign_shifted(rows(&raw).into_iter(), 3, b',', n.clone()));
        assert_eq!(out[3], vec!["4", "smith, john", "8"]);
        assert_eq!(n.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn an_ambiguous_shift_is_left_for_the_row_guard() {
        let n = Arc::new(AtomicU64::new(0));
        let raw: Vec<&[&str]> = vec![&["a", "b"], &["c", "d"], &["x", "y", "z"]];
        let out = collect(realign_shifted(rows(&raw).into_iter(), 2, b',', n.clone()));
        assert_eq!(out[2], vec!["x", "y", "z"]);
        assert_eq!(n.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_shift_that_breaks_typed_columns_is_not_applied() {
        let n = Arc::new(AtomicU64::new(0));
        let mut raw = profile();
        raw.push(&["5", "dave", "x", "y"]);
        let out = collect(realign_shifted(rows(&raw).into_iter(), 3, b',', n.clone()));
        assert_eq!(out[3], vec!["5", "dave", "x", "y"]);
        assert_eq!(n.load(Ordering::Relaxed), 0);
    }
}
