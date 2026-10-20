use std::fs::File;
use std::io::Read;

use anyhow::{Context, Result};

use crate::app::utils::ui;
use crate::args::AppArgs;
use crate::config;

mod damage;
mod encoding;
mod layout;

// Checks a csv/txt source for damage before it is converted: encoding,
// delimiter, quoting, header, rows split by line breaks or shifted by a
// delimiter inside a field. Every finding carries the evidence and, when
// there is one, the option that fixes it. Safe fixes are applied unless the
// user set that option or passed --no-auto-fix; the rest are suggested.

pub fn applies(args: &AppArgs) -> bool {
    matches!(args.input_file_type.as_str(), "csv" | "txt")
}

// One option change that repairs a finding.
#[derive(Clone, Debug, PartialEq)]
pub enum Fix {
    Encoding(String),
    FallbackEncoding(String),
    Delimiter(u8),
    NoHeader,
    NoQuote,
    JoinLines,
    Realign,
    TrailingDelimiter,
    FixMojibake(String),
    Trim(String),
}

impl Fix {
    // The option this fix sets, as clap names it.
    fn option_id(&self) -> &'static str {
        match self {
            Fix::Encoding(_) => "encoding",
            Fix::FallbackEncoding(_) => "fallback_encoding",
            Fix::Delimiter(_) => "delimiter",
            Fix::NoHeader => "no_header",
            Fix::NoQuote => "no_quote",
            Fix::JoinLines => "join_lines",
            Fix::Realign => "realign",
            Fix::TrailingDelimiter => "trailing_delimiter",
            Fix::FixMojibake(_) => "fix_mojibake",
            Fix::Trim(_) => "trim",
        }
    }

    pub fn apply(&self, args: &mut AppArgs) {
        match self {
            Fix::Encoding(enc) => args.encoding = Some(enc.clone()),
            Fix::FallbackEncoding(enc) => args.fallback_encoding = Some(enc.clone()),
            Fix::Delimiter(d) => args.delimiter = *d,
            Fix::NoHeader => args.no_header = true,
            Fix::NoQuote => args.no_quote = true,
            Fix::JoinLines => args.join_lines = true,
            Fix::Realign => args.realign = true,
            Fix::TrailingDelimiter => args.trailing_delimiter = true,
            Fix::FixMojibake(enc) => args.fix_mojibake = Some(enc.clone()),
            Fix::Trim(chars) => args.trim = Some(chars.clone()),
        }
    }

    // The command-line flags that request this fix.
    pub fn flags(&self) -> Vec<String> {
        let flag = |f: &str| vec![f.to_string()];
        let with = |f: &str, v: String| vec![f.to_string(), v];
        match self {
            Fix::Encoding(enc) => with("-e", enc.clone()),
            Fix::FallbackEncoding(enc) => with("--fallback-encoding", enc.clone()),
            Fix::Delimiter(d) => with("-d", delimiter_arg(*d)),
            Fix::NoHeader => flag("-H"),
            Fix::NoQuote => flag("--no-quote"),
            Fix::JoinLines => flag("--join-lines"),
            Fix::Realign => flag("--realign"),
            Fix::TrailingDelimiter => flag("--trailing-delimiter"),
            Fix::FixMojibake(enc) => with("--fix-mojibake", enc.clone()),
            Fix::Trim(chars) => with("--trim", chars.clone()),
        }
    }
}

fn delimiter_arg(d: u8) -> String {
    match d {
        b'\t' => "\\t".to_string(),
        other => char::from(other).to_string(),
    }
}

pub fn delimiter_label(d: u8) -> String {
    match d {
        b'\t' => "tab".to_string(),
        other => format!("'{}'", char::from(other)),
    }
}

#[derive(Debug)]
pub struct Finding {
    pub warn: bool,
    pub message: String,
    pub fix: Option<Fix>,
    // Safe fixes are unambiguous and applied automatically.
    pub safe: bool,
}

impl Finding {
    fn info(message: String) -> Self {
        Self {
            warn: false,
            message,
            fix: None,
            safe: false,
        }
    }

    fn warn(message: String) -> Self {
        Self {
            warn: true,
            message,
            fix: None,
            safe: false,
        }
    }

    fn safe_fix(message: String, fix: Fix) -> Self {
        Self {
            warn: true,
            message,
            fix: Some(fix),
            safe: true,
        }
    }

    fn suggest(message: String, fix: Fix) -> Self {
        Self {
            warn: true,
            message,
            fix: Some(fix),
            safe: false,
        }
    }
}

pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    // Applies the safe fixes the user did not override and returns them.
    pub fn apply_safe(&self, args: &mut AppArgs) -> Vec<Fix> {
        let mut applied = Vec::new();
        for fix in self
            .findings
            .iter()
            .filter(|f| f.safe)
            .filter_map(|f| f.fix.as_ref())
        {
            if !args.is_explicit(fix.option_id()) {
                fix.apply(args);
                applied.push(fix.clone());
            }
        }
        applied
    }

    // Fixes still worth trying: suggestions, and safe fixes that were not
    // applied (auto-fix off, or the user set that option differently).
    pub fn suggested(&self, applied: &[Fix]) -> Vec<Fix> {
        let mut out: Vec<Fix> = Vec::new();
        for fix in self.findings.iter().filter_map(|f| f.fix.as_ref()) {
            if !applied.contains(fix) && !out.contains(fix) {
                out.push(fix.clone());
            }
        }
        out
    }

    pub fn print(&self, applied: &[Fix]) {
        ui::section("doctor");
        for finding in &self.findings {
            let tail = match &finding.fix {
                Some(fix) if applied.contains(fix) => {
                    format!(" - auto-fix selected: {}", fix.flags().join(" "))
                }
                Some(fix) => format!(" - try: {}", fix.flags().join(" ")),
                None => String::new(),
            };
            let line = format!("{}{tail}", finding.message);
            if finding.warn {
                ui::warn(&line);
            } else {
                ui::info(&line);
            }
        }
        if self.findings.iter().all(|f| !f.warn) {
            ui::ok("no damage found in the sample");
        }
    }
}

// The conversion command as typed plus the given fixes, ready to copy.
pub fn command_with(fixes: &[Fix]) -> String {
    let words = rerun_args(std::env::args(), fixes);
    words
        .iter()
        .map(|w| shell_quote(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn rerun_args(args: impl IntoIterator<Item = String>, fixes: &[Fix]) -> Vec<String> {
    let mut words: Vec<String> = args
        .into_iter()
        .filter(|arg| arg != "--doctor" && arg != "--no-auto-fix")
        .collect();
    for fix in fixes {
        words.extend(fix.flags());
    }
    words
}

fn shell_quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

// Reads the start of the input, cut at the last complete line.
fn read_sample(path: &str) -> Result<(Vec<u8>, bool)> {
    let file = File::open(path).with_context(|| format!("failed to open {path:?}"))?;
    let mut bytes = Vec::with_capacity(config::DOCTOR_SAMPLE_BYTES);
    file.take(config::DOCTOR_SAMPLE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;

    let truncated = bytes.len() > config::DOCTOR_SAMPLE_BYTES;
    if truncated {
        bytes.truncate(config::DOCTOR_SAMPLE_BYTES);
        if let Some(end) = bytes.iter().rposition(|&b| b == b'\n') {
            bytes.truncate(end + 1);
        }
    }
    Ok((bytes, truncated))
}

pub fn examine(args: &AppArgs) -> Result<Report> {
    let (sample, truncated) = read_sample(&args.input_path)?;
    let mut findings = Vec::new();

    let decoded = encoding::examine(&sample, args, &mut findings)?;
    let layout = layout::examine(&decoded, args, &mut findings);
    damage::examine(&layout, &mut findings);

    let lines = layout.records.len();
    findings.push(Finding::info(if truncated {
        format!("checked the first {lines} lines (first 4 MiB of the file)")
    } else {
        format!("checked all {lines} lines")
    }));
    Ok(Report { findings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args(extra: &[&str]) -> AppArgs {
        let mut argv = vec!["DBMorph", "-i", "in.csv", "-o", "out"];
        argv.extend_from_slice(extra);
        let mut args = AppArgs::parse_from(&argv);
        args.explicit = extra
            .iter()
            .filter_map(|a| match *a {
                "-d" => Some("delimiter".to_string()),
                "--trim" => Some("trim".to_string()),
                _ => None,
            })
            .collect();
        args
    }

    fn report(findings: Vec<Finding>) -> Report {
        Report { findings }
    }

    #[test]
    fn safe_fixes_apply_unless_the_option_was_given() {
        let r = report(vec![
            Finding::safe_fix("d".into(), Fix::Delimiter(b';')),
            Finding::safe_fix("j".into(), Fix::JoinLines),
            Finding::suggest("r".into(), Fix::Realign),
        ]);

        let mut plain = args(&[]);
        assert_eq!(
            r.apply_safe(&mut plain),
            vec![Fix::Delimiter(b';'), Fix::JoinLines]
        );
        assert_eq!(plain.delimiter, b';');
        assert!(plain.join_lines && !plain.realign);

        let mut explicit = args(&["-d", ","]);
        assert_eq!(r.apply_safe(&mut explicit), vec![Fix::JoinLines]);
        assert_eq!(explicit.delimiter, b',');
        assert_eq!(
            r.suggested(&[Fix::JoinLines]),
            vec![Fix::Delimiter(b';'), Fix::Realign]
        );
    }

    #[test]
    fn every_fix_round_trips_through_its_flags() {
        let fixes = [
            Fix::Encoding("windows-1251".into()),
            Fix::FallbackEncoding("windows-1251".into()),
            Fix::Delimiter(b'\t'),
            Fix::Delimiter(b';'),
            Fix::NoHeader,
            Fix::NoQuote,
            Fix::JoinLines,
            Fix::Realign,
            Fix::TrailingDelimiter,
            Fix::FixMojibake("windows-1252".into()),
            Fix::Trim("\" ".into()),
        ];
        for fix in fixes {
            let mut by_fix = args(&[]);
            fix.apply(&mut by_fix);
            let flags = fix.flags();
            let refs: Vec<&str> = flags.iter().map(String::as_str).collect();
            let mut by_flags = args(&refs);
            by_flags.explicit.clear();
            assert_eq!(format!("{by_fix:?}"), format!("{by_flags:?}"), "{fix:?}");
        }
    }

    #[test]
    fn shell_quoting_keeps_simple_words_and_wraps_the_rest() {
        assert_eq!(shell_quote("--realign"), "--realign");
        assert_eq!(shell_quote("my file.csv"), "'my file.csv'");
        assert_eq!(shell_quote("\\t"), "'\\t'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn suggested_rerun_removes_doctor_flags_and_parses_for_conversion() {
        let args = [
            "DBMorph",
            "-i",
            "input.csv",
            "-o",
            "output",
            "-t",
            "csv",
            "--doctor",
            "--no-auto-fix",
        ]
        .into_iter()
        .map(str::to_string);
        let rerun = rerun_args(args, &[Fix::JoinLines]);

        assert!(
            !rerun
                .iter()
                .any(|arg| arg == "--doctor" || arg == "--no-auto-fix")
        );
        assert_eq!(rerun.last().map(String::as_str), Some("--join-lines"));
        let parsed = AppArgs::try_parse_from(rerun).expect("suggested command must parse");
        assert!(!parsed.doctor);
        assert!(parsed.join_lines);
        assert_eq!(parsed.database_type, "csv");
    }
}
