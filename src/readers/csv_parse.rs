use colored::*;
use csv::ReaderBuilder;
use csv::{self, StringRecord};
use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use tracing::info;

pub async fn csv_row_reader<P: AsRef<Path>>(
    csv_file: P,
    encoding: &String,
    delimiter: u8,
    remove_rows: Option<usize>,
) -> Result<impl Iterator<Item = Result<StringRecord, Box<dyn Error>>>, Box<dyn Error>> {
    // Reads a CSV file with specified encoding and delimiter, optionally skipping a number of rows.
    // Returns an iterator over the parsed CSV records as `StringRecord`.
    let path_ref = csv_file.as_ref();

    let file = File::open(path_ref)
        .map_err(|e| format!("Failed to open CSV file {:?}: {}", path_ref, e))?;

    let rust_encoding = Encoding::for_label(encoding.as_bytes())
        .ok_or_else(|| format!("Unsupported encoding: {}", encoding))?;

    let decoding_reader = DecodeReaderBytesBuilder::new()
        .encoding(Some(rust_encoding))
        .build(BufReader::new(file));

    let mut csv_reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(decoding_reader);

    if let Some(skip_rows) = remove_rows {
        for _ in 0..skip_rows {
            let _ = csv_reader.records().next();
        }
    }

    let iter = csv_reader
        .into_records()
        .map(|res| res.map_err(|e| Box::new(e) as Box<dyn Error>));

    Ok(iter)
}

pub async fn check_max_collumns<P: AsRef<Path>>(
    csv_file: P,
    encoding: &String,
    delimiter: u8,
    remove_rows: Option<usize>,
) -> Result<i32, Box<dyn Error>> {
    // Reads the first CSV row and prints the number of columns detected.
    // Returns the column count as i32, or an error if the file is empty.
    let mut all_rows = csv_row_reader(csv_file, encoding, delimiter, remove_rows)
        .await
        .unwrap();
    if let Some(result) = all_rows.next() {
        let record = result?;
        let columns_count = record.len();
        println!(
            "{}    Column count detected: {}",
            "✅ [INFO]".cyan().bold(),
            columns_count
        );

        drop(all_rows);

        Ok(columns_count.try_into().unwrap())
    } else {
        Err("Нет строк в CSV".into())
    }
}

pub async fn count_lines<P: AsRef<Path>>(
    path: P,
    remove_rows: Option<usize>,
) -> Result<usize, Box<dyn Error>> {
    // Counts the number of lines in a file, optionally skipping a given number of initial rows.
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let lines = reader.lines();

    let count = match remove_rows {
        Some(skip) => lines.skip(skip).count(),
        None => lines.count(),
    };

    Ok(count)
}
