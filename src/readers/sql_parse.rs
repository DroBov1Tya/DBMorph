use async_stream::stream;
use encoding_rs::Encoding;
use encoding_rs_io::DecodeReaderBytesBuilder;
use futures::Stream;
use regex::Regex;
use std::collections::BTreeSet;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};

use crate::utils::cli_prompt;

pub async fn parse_table_rows<'a>(
    file_path: &'a str,
    table_name: &'a str,
    encoding: &'a str,
) -> impl Stream<Item = Vec<String>> + Send + 'a {
    stream! {
        let file = File::open(file_path).expect("failed to open file");

        let rust_encoding = Encoding::for_label(encoding.as_bytes()).unwrap_or(encoding_rs::UTF_8);
        let decoding_reader = DecodeReaderBytesBuilder::new()
            .encoding(Some(rust_encoding))
            .build(BufReader::new(file));

        let reader = BufReader::new(decoding_reader);

        let insert_start_re = Regex::new(
            r#"(?i)^insert\s+(ignore\s+|or\s+\w+\s+)?into\s+[`"]?(\w+)[`"]?\s*(\([^)]+\))?\s*values\s*"#,
        ).unwrap();
        let tuple_re = Regex::new(r"\(([^()]*)\)").unwrap();

        let mut inside_insert = false;
        let mut current_table = String::new();
        let mut current_values_buf = String::new();

        for line in reader.lines() {
            let line = line.unwrap();
            let trimmed = line.trim();

            if !inside_insert {
                if let Some(caps) = insert_start_re.captures(trimmed) {
                    current_table = caps.get(2).unwrap().as_str().to_string();

                    if current_table != table_name {
                        continue;
                    }

                    inside_insert = true;

                    if let Some(pos) = trimmed.to_lowercase().find("values") {
                        let values_part = &trimmed[pos + 6..];
                        current_values_buf.push_str(values_part);
                        current_values_buf.push(' ');
                    }
                }
            } else {
                current_values_buf.push_str(trimmed);
                current_values_buf.push(' ');

                if trimmed.ends_with(';') {
                    current_values_buf = current_values_buf.trim_end_matches(';').to_string();

                    for tuple_cap in tuple_re.captures_iter(&current_values_buf) {
                        let row_str = tuple_cap.get(1).unwrap().as_str();
                        let row: Vec<String> = row_str
                            .split(',')
                            .map(|v| v.trim().trim_matches('\'').to_string())
                            .collect();

                        yield row;
                    }

                    inside_insert = false;
                    current_table.clear();
                    current_values_buf.clear();
                }
            }
        }
    }
}

pub async fn extract_table_names(file_path: &String) -> Result<String, Box<dyn Error>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let insert_re = Regex::new(r#"(?i)^insert\s+(ignore\s+|or\s+\w+\s+)?into\s+[`"]?(\w+)[`"]?"#)?;

    let mut table_names = BTreeSet::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        if let Some(caps) = insert_re.captures(trimmed) {
            if let Some(table_name) = caps.get(2) {
                table_names.insert(table_name.as_str().to_string());
            }
        }
    }

    let choose_table = cli_prompt::select_table(&table_names).await?;

    Ok(choose_table)
}

pub async fn count_rows_in_table(
    file_path: &String,
    target_table: &String,
) -> Result<u32, Box<dyn Error>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let insert_start_re = Regex::new(
        r#"(?i)^insert\s+(ignore\s+|or\s+\w+\s+)?into\s+[`"]?(\w+)[`"]?\s*(\([^)]+\))?\s*values\s*"#,
    )?;
    let tuple_re = Regex::new(r"\(([^()]*)\)")?;

    let mut current_values_buf = String::new();
    let mut inside_insert = false;
    let mut current_table = String::new();
    let mut count_rows = 0;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        if let Some(caps) = insert_start_re.captures(trimmed) {
            current_table = caps
                .get(2)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            if current_table == target_table.clone() {
                inside_insert = true;
                current_values_buf.clear();
                current_values_buf.push_str(trimmed);
                current_values_buf.push(' ');
            }
            continue;
        }

        if inside_insert {
            current_values_buf.push_str(trimmed);
            current_values_buf.push(' ');

            if trimmed.ends_with(';') {
                inside_insert = false;

                for _tuple_cap in tuple_re.captures_iter(&current_values_buf) {
                    count_rows += 1;
                }

                current_values_buf.clear();
            }
        }
    }

    Ok(count_rows)
}

pub async fn count_columns_in_first_row(
    file_path: &str,
    target_table: &str,
) -> Result<i32, Box<dyn Error>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let insert_start_re = Regex::new(
        r#"(?i)^insert\s+(ignore\s+|or\s+\w+\s+)?into\s+[`"]?(\w+)[`"]?\s*(\([^)]+\))?\s*values\s*"#,
    )?;
    let tuple_re = Regex::new(r"\(([^()]*)\)")?;

    let mut current_values_buf = String::new();
    let mut inside_insert = false;
    let mut current_table = String::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        if !inside_insert {
            if let Some(caps) = insert_start_re.captures(trimmed) {
                current_table = caps.get(2).unwrap().as_str().to_string();
                if current_table == target_table {
                    inside_insert = true;

                    if let Some(pos) = trimmed.to_lowercase().find("values") {
                        let values_part = &trimmed[pos + 6..];
                        current_values_buf.push_str(values_part);
                        current_values_buf.push(' ');
                    }
                }
                continue;
            }
        } else {
            current_values_buf.push_str(trimmed);
            current_values_buf.push(' ');

            if trimmed.ends_with(';') {
                if let Some(first_tuple_cap) = tuple_re.captures(&current_values_buf) {
                    let row_str = first_tuple_cap.get(1).unwrap().as_str();
                    let columns_count = row_str.split(',').count() as i32;
                    return Ok(columns_count);
                } else {
                    return Err("No tuples found in VALUES".into());
                }
            }
        }
    }

    Err("No matching INSERT found for the specified table".into())
}
