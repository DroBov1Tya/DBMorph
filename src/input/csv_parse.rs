use std::io::BufReader;
use std::path::Path;
use std::fs::File;
use std::error::Error;
use csv::{self, StringRecord};
use csv::ReaderBuilder;
use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;
use tracing::info;


pub async fn csv_row_reader<P: AsRef<Path>>(csv_file: P, delimiter: u8, encoding: String) -> Result<impl Iterator<Item = Result<StringRecord, Box<dyn Error>>>, Box<dyn Error>> {
    let path_ref = csv_file.as_ref();
    
    let file = File::open(path_ref)
        .map_err(|e| format!("Failed to open CSV file {:?}: {}", path_ref, e))?;

    let rust_encoding = Encoding::for_label(encoding.as_bytes())
        .ok_or_else(|| format!("Unsupported encoding: {}", encoding))?;

    let decoding_reader = DecodeReaderBytesBuilder::new()
        .encoding(Some(rust_encoding))
        .build(BufReader::new(file));

    let csv_reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(true)
        .flexible(true)
        .from_reader(decoding_reader);

    let iter = csv_reader
        .into_records()
        .map(|res| res.map_err(|e| Box::new(e) as Box<dyn Error>));

    Ok(iter)
}

pub async fn check_max_collumns<P: AsRef<Path>>(
    csv_file: P,
    delimiter: u8,
    encoding: String
) -> Result<i32, Box<dyn Error>> {
    let mut all_rows = csv_row_reader(csv_file, delimiter, encoding).await.unwrap();
    if let Some(result) = all_rows.next() {
        let record = result?;
        let columns_count = record.len();
        println!("[+] Определено количество колонок: {}", columns_count);
        
        drop(all_rows);

        Ok(columns_count.try_into().unwrap())
    } else {
        Err("Нет строк в CSV".into())
    }
}