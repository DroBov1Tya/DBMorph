use std::error::Error;
use colored::*;
use csv::StringRecord;
use sqlx::{sqlite::SqliteQueryResult, SqliteConnection};

use crate::utils::cli_prompt::process_and_pause;
use crate::utils::output_format;
use super::sqlite_init;

pub async fn init_insert_process(
    conn: &mut SqliteConnection,
    table_name: &String,
    columns: Vec<String>,
    batch_size: Option<u32>,
    all_rows: impl Iterator<Item = Result<StringRecord, Box<dyn std::error::Error>>>,
    lines_count: usize
) -> Result<(), Box<dyn Error>> {
    let mode: (String,) = sqlx::query_as("PRAGMA journal_mode = OFF;")
        .fetch_one(&mut *conn)
        .await?;
    println!(
        "{}   PRAGMA journal_mode set to: {}",
        "✅ [DB]".green().bold(),
        mode.0
    );

    sqlx::query("PRAGMA synchronous = OFF;")
        .execute(&mut *conn)
        .await?;

    let preview_count = 5;
    let batch_size = batch_size.unwrap().try_into().unwrap();
    let mut preview_shown = false;
    let mut total_rows = 0;

    let mut chunk: Vec<Vec<String>> = Vec::with_capacity(batch_size);

    for row_result in all_rows {
        match row_result {
            Ok(record) => {
                let fields: Vec<String> = record.iter().map(|s| s.to_string()).collect();
                chunk.push(fields);
                total_rows += 1;

                if !preview_shown && chunk.len() == preview_count {
                    let preview = chunk[..preview_count].to_vec();
                    let _ = process_and_pause(preview).await;
                    preview_shown = true;
                }

                if chunk.len() >= batch_size {
                    sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk).await?;
                    chunk.clear();

                    output_format::sqlite_processing(total_rows, lines_count).await;
                }
            }
            Err(e) => {
                eprintln!(
                    "{}   Failed to read row from CSV: {}",
                    "🚫 [ERROR]".red().bold(),
                    e
                );
            }
        }
    }

    if !chunk.is_empty() {
        sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk).await?;
        output_format::sqlite_processing_finish(total_rows).await;
        chunk.clear();
    }

    Ok(())
}