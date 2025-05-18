use std::fs::File;
use std::error::Error;
use std::io::{self, Read};
use std::path::Path;
use chardetng::EncodingDetector;

pub fn detect_encoding<P: AsRef<Path>>(file_path: P) -> Result<String, Box<dyn Error>> {
    let sample_size = 10000;
    let f = File::open(file_path)?;
    let mut buffer = Vec::with_capacity(sample_size);
    f.take(sample_size as u64).read_to_end(&mut buffer)?;

    if buffer.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Ok("utf-8".to_string());
    } else if buffer.starts_with(&[0xFF, 0xFE]) {
        return Ok("utf-16le".to_string());
    } else if buffer.starts_with(&[0xFE, 0xFF]) {
        return Ok("utf-16be".to_string());
    }

    let mut detector = EncodingDetector::new();
    detector.feed(&buffer, true);
    let encoding_guess = detector.guess(None, true);
    Ok(encoding_guess.name().to_lowercase())
}