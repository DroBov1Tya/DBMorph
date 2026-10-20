use anyhow::{Context, Result};
use encoding_rs::Encoding;

use crate::args::AppArgs;

use super::{Finding, Fix};

// Detect a supported text encoding and decode the bounded doctor sample.
pub fn examine(sample: &[u8], args: &AppArgs, findings: &mut Vec<Finding>) -> Result<String> {
    let detected = match args.encoding.as_deref() {
        Some(explicit) => explicit.to_string(),
        None => {
            // chardetng's detector uses the same bounded sample as this pass.
            let mut detector = chardetng::EncodingDetector::new();
            detector.feed(sample, true);
            detector.guess(None, true).name().to_lowercase()
        }
    };
    if args.encoding.is_none()
        && std::str::from_utf8(sample).is_ok()
        && Encoding::for_label(detected.as_bytes()).is_some_and(|enc| enc != encoding_rs::UTF_8)
    {
        findings.push(Finding::safe_fix(
            "sample is valid UTF-8; detector chose a legacy encoding".into(),
            Fix::Encoding("utf-8".into()),
        ));
    }
    let encoding = Encoding::for_label(detected.as_bytes())
        .with_context(|| format!("unsupported detected encoding: {detected}"))?;
    if args.encoding.is_none() && encoding == encoding_rs::UTF_8 {
        findings.push(Finding::info("input is valid UTF-8".to_string()));
    }
    let (text, _, had_errors) = encoding.decode(sample);
    if had_errors {
        if let Some(fallback) = args.fallback_encoding.as_deref() {
            findings.push(Finding::info(format!(
                "invalid bytes decoded with {fallback}"
            )));
        } else if let Some(fix) = fallback_fix(&detected, sample) {
            findings.push(Finding::suggest(
                format!("sample contains bytes invalid in {detected}"),
                fix,
            ));
        } else {
            findings.push(Finding::warn(format!(
                "sample contains bytes invalid in {detected}; choose a supported single-byte --fallback-encoding"
            )));
        }
    }
    Ok(text.into_owned())
}

fn fallback_fix(primary: &str, sample: &[u8]) -> Option<Fix> {
    let primary = Encoding::for_label(primary.as_bytes())?;
    if primary != encoding_rs::UTF_8 {
        return None;
    }

    // Estimate the fallback from only the lines that fail UTF-8 validation;
    // the main detector has already identified UTF-8 as the primary encoding.
    let mut detector = chardetng::EncodingDetector::new();
    let mut found_invalid_line = false;
    for line in sample.split_inclusive(|byte| *byte == b'\n') {
        if std::str::from_utf8(line).is_err() {
            detector.feed(line, false);
            found_invalid_line = true;
        }
    }
    if !found_invalid_line {
        return None;
    }
    let fallback = detector.guess(None, true);
    (fallback != encoding_rs::UTF_8 && fallback.is_single_byte())
        .then(|| Fix::FallbackEncoding(fallback.name().to_lowercase()))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn fallback_suggestions_are_cli_accepted_single_byte_encodings() {
        let args =
            AppArgs::try_parse_from(["DBMorph", "-i", "input.csv", "-o", "output", "-e", "utf-8"])
                .unwrap();
        let mut findings = Vec::new();
        let sample = encoding_rs::WINDOWS_1252
            .encode("Café déjà vu. Français, élève, cœur, naïve. ")
            .0
            .into_owned();
        examine(&sample, &args, &mut findings).unwrap();
        let fix = findings
            .iter()
            .find_map(|finding| finding.fix.as_ref())
            .expect("UTF-8 decode errors should produce a supported fallback suggestion");
        assert!(matches!(fix, Fix::FallbackEncoding(_)));
        let flags = fix.flags();
        let mut argv = vec!["DBMorph", "-i", "input.csv", "-o", "output"];
        argv.extend(flags.iter().map(String::as_str));
        AppArgs::try_parse_from(argv).expect("generated fallback suggestion must parse");
    }

    #[test]
    fn non_utf8_primary_does_not_suggest_an_ineffective_fallback() {
        let args = AppArgs::try_parse_from([
            "DBMorph",
            "-i",
            "input.csv",
            "-o",
            "output",
            "-e",
            "utf-16le",
        ])
        .unwrap();
        let mut findings = Vec::new();
        examine(&[0xff], &args, &mut findings).unwrap();
        assert!(
            findings
                .iter()
                .any(|finding| finding.warn && finding.fix.is_none())
        );
        assert!(
            !findings
                .iter()
                .filter_map(|finding| finding.fix.as_ref())
                .any(|fix| matches!(fix, Fix::FallbackEncoding(enc) if enc == "utf-8"))
        );
    }
}
