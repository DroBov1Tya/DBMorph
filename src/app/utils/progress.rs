use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// Progress of one run. File sources count the bytes the reader has consumed
// against the file size; sources with a known row count (Parquet, a counted
// SQLite table) use rows instead. The byte position runs at most one pipeline
// batch ahead of the writer, which is invisible in a percentage.
#[derive(Clone)]
pub struct Progress {
    started: Instant,
    bytes_read: Arc<AtomicU64>,
    // Bytes already read when the clock started, for the byte rate.
    bytes_at_start: u64,
    total: Total,
}

#[derive(Copy, Clone)]
enum Total {
    Bytes(u64),
    Rows(u64),
    Unknown,
}

impl Progress {
    pub fn unknown() -> Self {
        Self::with_total(Total::Unknown)
    }

    pub fn rows(total: u64) -> Self {
        Self::with_total(Total::Rows(total))
    }

    // Percentage by bytes of the given file; unknown when its size is not.
    pub fn file(path: impl AsRef<Path>) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() > 0 => Self::with_total(Total::Bytes(meta.len())),
            _ => Self::unknown(),
        }
    }

    fn with_total(total: Total) -> Self {
        Self {
            started: Instant::now(),
            bytes_read: Arc::new(AtomicU64::new(0)),
            bytes_at_start: 0,
            total,
        }
    }

    // Same counters with the clock reset: the writer calls this when rows
    // start flowing, so time spent at the preview prompt does not skew the
    // rate and ETA.
    pub fn restarted(&self) -> Self {
        Self {
            started: Instant::now(),
            bytes_at_start: self.bytes_read.load(Ordering::Relaxed),
            ..self.clone()
        }
    }

    // Wraps the raw source so every byte read is counted.
    pub fn reader<R: Read>(&self, inner: R) -> CountingReader<R> {
        CountingReader {
            inner,
            read: self.bytes_read.clone(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    // Share of the work done, 0.0..=1.0, when the total is known.
    pub fn fraction(&self, rows_done: u64) -> Option<f64> {
        let (done, total) = match self.total {
            Total::Bytes(total) => (self.bytes_read.load(Ordering::Relaxed), total),
            Total::Rows(total) => (rows_done, total),
            Total::Unknown => return None,
        };
        (total > 0).then(|| (done as f64 / total as f64).min(1.0))
    }

    pub fn rows_per_sec(&self, rows_done: u64) -> u64 {
        let secs = self.elapsed().as_secs_f64();
        if secs > 0.0 {
            (rows_done as f64 / secs) as u64
        } else {
            0
        }
    }

    // Remaining time at the average rate since the clock started.
    pub fn eta(&self, rows_done: u64) -> Option<Duration> {
        let elapsed = self.elapsed().as_secs_f64();
        let (done, left) = match self.total {
            Total::Bytes(total) => {
                let read = self.bytes_read.load(Ordering::Relaxed);
                (
                    read.saturating_sub(self.bytes_at_start),
                    total.saturating_sub(read),
                )
            }
            Total::Rows(total) => (rows_done, total.saturating_sub(rows_done)),
            Total::Unknown => return None,
        };
        if done == 0 || elapsed <= 0.0 {
            return None;
        }
        Duration::try_from_secs_f64(elapsed * left as f64 / done as f64).ok()
    }
}

pub struct CountingReader<R> {
    inner: R,
    read: Arc<AtomicU64>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_bytes_through_the_reader() {
        let progress = Progress::unknown();
        let mut out = Vec::new();
        progress
            .reader(&b"hello world"[..])
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(progress.bytes_read.load(Ordering::Relaxed), 11);
        assert_eq!(progress.fraction(5), None);
    }

    #[test]
    fn byte_fraction_follows_the_reader_and_caps_at_one() {
        let progress = Progress::with_total(Total::Bytes(4));
        let mut buf = [0u8; 2];
        let mut reader = progress.reader(&b"abcdef"[..]);
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(progress.fraction(0), Some(0.5));
        reader.read_exact(&mut [0u8; 4]).unwrap();
        assert_eq!(progress.fraction(0), Some(1.0));
    }

    #[test]
    fn row_fraction_and_eta() {
        let progress = Progress::rows(200);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(progress.fraction(50), Some(0.25));
        assert!(progress.eta(0).is_none());
        assert!(progress.eta(50).is_some());
        assert_eq!(progress.eta(200), Some(Duration::ZERO));
        assert_eq!(Progress::rows(0).fraction(10), None);
    }

    #[test]
    fn byte_eta_ignores_bytes_read_before_the_restart() {
        let progress = Progress::with_total(Total::Bytes(1_000));
        let mut reader = progress.reader(&[0u8; 1_000][..]);
        reader.read_exact(&mut [0u8; 600]).unwrap();
        let writing = progress.restarted();
        assert!(writing.eta(0).is_none(), "no bytes since the restart yet");
        std::thread::sleep(Duration::from_millis(5));
        reader.read_exact(&mut [0u8; 100]).unwrap();
        let eta = writing.eta(0).unwrap();
        // 300 bytes left at 100 bytes per elapsed: about 3x the elapsed time.
        assert!(eta >= writing.elapsed() * 2, "{eta:?}");
    }

    #[test]
    fn absurd_totals_do_not_panic() {
        let progress = Progress::rows(u64::MAX);
        std::thread::sleep(Duration::from_millis(2));
        let _ = progress.eta(1);
    }

    #[test]
    fn missing_or_empty_file_has_no_total() {
        assert_eq!(Progress::file("/definitely/not/here").fraction(1), None);
    }
}
