use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{bail, Context, Result};
use csv::ReaderBuilder;
use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;

use crate::config;

pub async fn csv_row_reader<P: AsRef<Path>>(
    csv_file: P,
    delimiter: u8,
    encoding: &str,
    has_headers: bool,
) -> Result<impl Iterator<Item = Result<Vec<String>>>> {
    let path = csv_file.as_ref();
    let file = File::open(path).with_context(|| format!("failed to open CSV {path:?}"))?;

    let rust_encoding = Encoding::for_label(encoding.as_bytes())
        .with_context(|| format!("unsupported encoding: {encoding}"))?;

    let decoding_reader = DecodeReaderBytesBuilder::new()
        .encoding(Some(rust_encoding))
        .build(BufReader::with_capacity(config::READ_BUFFER_BYTES, file));

    let csv_reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(has_headers)
        .flexible(true)
        .from_reader(decoding_reader);

    let iter = csv_reader.into_records().map(|res| {
        res.map(|record| record.iter().map(|s| s.to_string()).collect::<Vec<String>>())
            .map_err(anyhow::Error::from)
    });

    Ok(iter)
}

/// Returns column names for the input. With a header row the real names are
/// taken from the first line; otherwise synthetic `c0..cN` names are generated
/// from the width of the first data row.
pub async fn csv_headers<P: AsRef<Path>>(
    csv_file: P,
    delimiter: u8,
    encoding: &str,
    has_headers: bool,
) -> Result<Vec<String>> {
    let path = csv_file.as_ref();
    let file = File::open(path).with_context(|| format!("failed to open CSV {path:?}"))?;

    let rust_encoding = Encoding::for_label(encoding.as_bytes())
        .with_context(|| format!("unsupported encoding: {encoding}"))?;

    let decoding_reader = DecodeReaderBytesBuilder::new()
        .encoding(Some(rust_encoding))
        .build(BufReader::with_capacity(config::READ_BUFFER_BYTES, file));

    let mut csv_reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(decoding_reader);

    let mut first = csv::StringRecord::new();
    if !csv_reader.read_record(&mut first)? {
        bail!("CSV file has no rows");
    }

    if has_headers {
        Ok(first.iter().map(|s| s.to_string()).collect())
    } else {
        Ok((0..first.len()).map(|i| format!("c{i}")).collect())
    }
}
