use crate::app::transform::infer;

use super::layout::Layout;
use super::{Finding, Fix};

pub fn examine(layout: &Layout, findings: &mut Vec<Finding>) {
    let width = layout.width;
    if width == 0 {
        findings.push(Finding::warn("no delimited records found in sample".into()));
        return;
    }
    let clean: Vec<Vec<String>> = layout
        .records
        .iter()
        .filter(|row| row.len() == width)
        .take(crate::config::REALIGN_PROFILE_ROWS)
        .cloned()
        .collect();
    let types = infer::infer_types(&clean, width);
    let cell_count = layout.records.iter().map(Vec::len).sum::<usize>();
    let literal_quotes = layout
        .records
        .iter()
        .flatten()
        .filter(|cell| cell.len() >= 2 && cell.starts_with('"') && cell.ends_with('"'))
        .count();
    if cell_count > 0 && literal_quotes * 2 >= cell_count {
        findings.push(Finding::suggest(
            "many values retain literal quote characters".into(),
            Fix::Trim("\"".into()),
        ));
    }
    let mojibake = layout.records.iter().flatten().any(|cell| {
        cell.contains("Ã") || cell.contains("Â") || cell.contains("Ð") || cell.contains("Ñ")
    });
    if mojibake {
        let encoding = if layout
            .records
            .iter()
            .flatten()
            .any(|cell| cell.contains("Ð") || cell.contains("Ñ"))
        {
            "windows-1251"
        } else {
            "windows-1252"
        };
        findings.push(Finding::suggest(
            "some values resemble text decoded with the wrong single-byte encoding".into(),
            Fix::FixMojibake(encoding.into()),
        ));
    }
    let shifted = layout.records.iter().any(|row| {
        row.len() > width
            && (1..=crate::config::MAX_REALIGN_EXTRA).contains(&(row.len() - width))
            && (0..width)
                .filter(|&at| types[at] == infer::ColType::Text)
                .count()
                > 0
    });
    if shifted {
        findings.push(Finding::suggest(
            "some rows contain extra fields that may have shifted columns".into(),
            Fix::Realign,
        ));
    }
}
