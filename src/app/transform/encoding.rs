use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use chardetng::EncodingDetector;

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
