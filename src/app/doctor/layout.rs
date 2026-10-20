use crate::app::readers::csv_parse::{self, CsvOptions};
use crate::args::AppArgs;

use super::{Finding, Fix};

pub struct Layout {
    pub records: Vec<Vec<String>>,
    pub width: usize,
}

pub fn examine(text: &str, args: &AppArgs, findings: &mut Vec<Finding>) -> Layout {
    let candidates = [args.delimiter, b',', b'\t', b';', b'|'];
    let mut best = (Vec::new(), args.delimiter, 0usize);
    for delimiter in candidates {
        let opts = CsvOptions {
            delimiter,
            encoding: "utf-8".into(),
            fallback: None,
            has_headers: !args.no_header,
            quoting: !args.no_quote,
            repair: !args.no_repair,
        };
        let records = csv_parse::parse_sample(text, &opts, crate::config::DOCTOR_SAMPLE_LINES);
        let widths: Vec<usize> = records.iter().map(Vec::len).collect();
        let mode_width = widths
            .iter()
            .copied()
            .filter(|w| *w > 1)
            .max_by_key(|w| widths.iter().filter(|other| *other == w).count())
            .unwrap_or(1);
        let consistency = widths.iter().filter(|w| **w == mode_width).count();
        let score = if mode_width > 1 { consistency } else { 0 };
        if score > best.2 {
            best = (records, delimiter, score);
        }
    }
    let (mut records, delimiter, _) = best;
    if delimiter != args.delimiter {
        findings.push(Finding::safe_fix(
            format!(
                "detected {} as the consistent delimiter",
                super::delimiter_label(delimiter)
            ),
            Fix::Delimiter(delimiter),
        ));
    }
    if !args.no_header
        && records
            .first()
            .is_some_and(|row| row.last().is_some_and(String::is_empty))
    {
        records[0].pop();
    }
    let width = records.first().map_or(0, Vec::len);
    if !args.no_header && records.len() > 1 && width > 0 {
        let inferred = crate::app::transform::infer::infer_types(&records[1..], width);
        let first_looks_like_data = records[0]
            .iter()
            .zip(&inferred)
            .filter(|(cell, ty)| crate::app::transform::infer::coerce(**ty, cell).is_some())
            .count();
        if first_looks_like_data == width {
            findings.push(Finding::suggest(
                "the first row has the same value types as the data rows".into(),
                Fix::NoHeader,
            ));
        }
    }
    if !args.no_quote && text.contains('"') {
        let unquoted = CsvOptions {
            quoting: false,
            ..CsvOptions {
                delimiter,
                encoding: "utf-8".into(),
                fallback: None,
                has_headers: !args.no_header,
                quoting: true,
                repair: false,
            }
        };
        let rows = csv_parse::parse_sample(text, &unquoted, crate::config::DOCTOR_SAMPLE_LINES);
        if rows.len() > records.len()
            && rows.iter().map(Vec::len).max() == Some(width)
            && rows.iter().all(|row| row.len() == width)
        {
            findings.push(Finding::suggest(
                "disabling quote handling gives a more consistent row layout".into(),
                Fix::NoQuote,
            ));
        }
    }
    if records
        .iter()
        .any(|row| row.len() + 1 == width && !row.is_empty())
    {
        findings.push(Finding::suggest(
            "some rows have fewer fields than the header".into(),
            Fix::JoinLines,
        ));
    }
    let has_trailing = records
        .iter()
        .any(|row| row.len() == width + 1 && row.last().is_some_and(String::is_empty));
    if has_trailing {
        findings.push(Finding::safe_fix(
            "rows contain a trailing delimiter".into(),
            Fix::TrailingDelimiter,
        ));
        for row in &mut records {
            if row.len() == width + 1 && row.last().is_some_and(String::is_empty) {
                row.pop();
            }
        }
    }
    Layout { records, width }
}
