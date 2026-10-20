use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use chardetng::EncodingDetector;
use encoding_rs::Encoding;

pub fn detect_encoding<P: AsRef<Path>>(file_path: P) -> Result<String> {
    let sample_size = 10_000;
    let path = file_path.as_ref();
    let file = File::open(path).with_context(|| format!("failed to open {path:?}"))?;

    let mut buffer = Vec::with_capacity(sample_size);
    file.take(sample_size as u64).read_to_end(&mut buffer)?;

    if buffer.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Ok("utf-8".to_string());
    } else if buffer.starts_with(&[0xFF, 0xFE]) {
        return Ok("utf-16le".to_string());
    } else if buffer.starts_with(&[0xFE, 0xFF]) {
        return Ok("utf-16be".to_string());
    }

    let mut detector = EncodingDetector::new();
    detector.feed(&buffer, true);
    let guess = detector.guess(None, true);
    Ok(guess.name().to_lowercase())
}

// Undoes mojibake: text that was UTF-8 but got decoded as the single-byte
// encoding `wrong` ("Ã©" for "é" with windows-1252). Encoding it back must be
// lossless and give valid UTF-8, otherwise the text was never mojibake.
pub fn unmojibake(text: &str, wrong: &'static Encoding) -> Option<String> {
    if text.is_ascii() {
        return None;
    }
    let (bytes, _, had_errors) = wrong.encode(text);
    if had_errors {
        return None;
    }
    let fixed = std::str::from_utf8(&bytes).ok()?;
    (fixed != text).then(|| fixed.to_string())
}

// Encoding by label; the argument parser has already validated it.
pub fn by_label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::{WINDOWS_1251, WINDOWS_1252};

    #[test]
    fn mojibake_from_windows_1252_is_undone() {
        assert_eq!(unmojibake("CafÃ©", WINDOWS_1252).as_deref(), Some("Café"));
    }

    #[test]
    fn mojibake_from_windows_1251_is_undone() {
        let broken = WINDOWS_1251.decode("Привет".as_bytes()).0;
        assert_eq!(unmojibake(&broken, WINDOWS_1251).as_deref(), Some("Привет"));
    }

    #[test]
    fn correct_text_is_left_alone() {
        for good in ["Café", "Привет", "plain ascii", "日本語"] {
            assert_eq!(unmojibake(good, WINDOWS_1252), None, "{good}");
            assert_eq!(unmojibake(good, WINDOWS_1251), None, "{good}");
        }
    }
}
