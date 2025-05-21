use colored::*;
use csv::StringRecord;
use sqlx::SqliteConnection;
use std::error::Error;

use super::sqlite_init;
use crate::utils::cli_prompt::process_and_pause;
use crate::utils::output_format;

pub async fn init_insert_process(
    conn: &mut SqliteConnection,
    table_name: &String,
    columns: Vec<String>,
    batch_size: Option<u32>,
    all_rows: impl Iterator<Item = Result<StringRecord, Box<dyn std::error::Error>>>,
    total_rows: usize,
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
    let mut lines_count = 0;
    let progress_bar = output_format::init_progress_bar(total_rows as u64).await?;

    let mut chunk: Vec<Vec<String>> = Vec::with_capacity(batch_size);

    for row_result in all_rows {
        match row_result {
            Ok(record) => {
                let fields: Vec<String> = record.iter().map(|s| s.to_string()).collect();
                chunk.push(fields);
                lines_count += 1;

                if !preview_shown && chunk.len() == preview_count {
                    let preview = chunk[..preview_count].to_vec();
                    let _ = process_and_pause(preview).await;
                    preview_shown = true;
                }

                if chunk.len() >= batch_size {
                    progress_bar.set_position(lines_count);
                    sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk)
                        .await?;
                    chunk.clear();

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
        progress_bar.finish_with_message(format!("{} {} Rows processed", "✅ [DONE]".green().bold(), &lines_count));
        chunk.clear();
    }

    Ok(())
}
