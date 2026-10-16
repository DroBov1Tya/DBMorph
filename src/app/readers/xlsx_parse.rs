use std::path::Path;

use anyhow::{Context, Result};
use calamine::{Data, Reader, open_workbook_auto};

fn first_sheet_name(path: &Path) -> Result<String> {
    let workbook =
        open_workbook_auto(path).with_context(|| format!("failed to open workbook {path:?}"))?;
    workbook
        .sheet_names()
        .first()
        .cloned()
        .context("workbook has no sheets")
}

fn load_rows(path: &Path) -> Result<Vec<Vec<String>>> {
    let mut workbook =
        open_workbook_auto(path).with_context(|| format!("failed to open workbook {path:?}"))?;
    let sheet = first_sheet_name(path)?;
    let range = workbook
        .worksheet_range(&sheet)
        .with_context(|| format!("failed to read sheet {sheet:?}"))?;

    let width = range.width();
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(range.height());
    for row in range.rows() {
        if row.iter().all(|cell| matches!(cell, Data::Empty)) {
            continue;
        }
        let mut record: Vec<String> = row.iter().map(cell_to_string).collect();
        record.resize(width, String::new());
        rows.push(record);
    }
    Ok(rows)
}

// Column names from the first sheet: header row when present, else c0, c1 and so on.
pub fn xlsx_headers(path: &Path, has_headers: bool) -> Result<Vec<String>> {
    let rows = load_rows(path)?;
    let first = rows.first().context("workbook sheet has no rows")?;
    if has_headers {
        Ok(first
            .iter()
            .enumerate()
            .map(|(i, name)| {
                if name.is_empty() {
                    format!("c{i}")
                } else {
                    name.clone()
                }
            })
            .collect())
    } else {
        Ok((0..first.len()).map(|i| format!("c{i}")).collect())
    }
}

// Streams first-sheet rows as strings; drops the header row when present.
pub fn xlsx_row_reader(
    path: &Path,
    has_headers: bool,
) -> Result<impl Iterator<Item = Result<Vec<String>>>> {
    let rows = load_rows(path)?;
    let mut iter = rows.into_iter();
    if has_headers {
        iter.next();
    }
    Ok(iter.map(Ok))
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Bool(b) => b.to_string(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => format_float(*f),
        Data::DateTime(dt) => dt
            .as_datetime()
            .map(|d| d.to_string())
            .unwrap_or_else(|| dt.as_f64().to_string()),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => bail_string(e),
    }
}

fn format_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        f.to_string()
    }
}

fn bail_string(err: &calamine::CellErrorType) -> String {
    format!("#ERROR:{err:?}")
}
