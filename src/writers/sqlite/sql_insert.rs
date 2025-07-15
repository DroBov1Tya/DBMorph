use colored::*;
use futures::pin_mut;
use futures::StreamExt;
use sqlx::SqliteConnection;
use std::error::Error;

use crate::readers::sql_parse;
use crate::utils::cli_prompt::process_and_pause;
use crate::utils::output_format;
use super::sqlite_init;

pub async fn init_sql(
    conn: &mut SqliteConnection,
    input_file: String,
    table_name: String,
    target_table_name: String,
    columns: Vec<String>,
    encoding: String,
    batch_size: Option<u32>,
    total_rows: u32,
) -> Result<(), Box<dyn Error>> {
    // Sets SQLite PRAGMA options for faster inserts
    // Parses rows from a file into a stream with specified encoding
    // Collects rows into batches and inserts them into SQLite table in chunks
    // Shows a preview of the first 5 rows before continuing
    // Updates progress bar while processing rows
    // Inserts any remaining rows after stream ends and finishes progress bar
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

    let stream = sql_parse::parse_table_rows(&input_file, &target_table_name, &encoding).await;

    pin_mut!(stream);

    let max_chunk_size = batch_size.unwrap_or(1000);
    let preview_count = 5;
    let mut chunk: Vec<Vec<String>> = Vec::new();
    let mut preview_shown = false;
    let mut lines_count = 0;
    let progress_bar = output_format::init_progress_bar(total_rows as u64).await?;

    while let Some(row) = stream.next().await {
        chunk.push(row);
        lines_count += 1;

        if !preview_shown && chunk.len() == preview_count {
            let preview = chunk[..preview_count].to_vec();
            let _ = process_and_pause(preview).await;
            preview_shown = true;
        }

        if chunk.len() as u32 >= max_chunk_size {
            progress_bar.set_position(lines_count);
            let _ =
                sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk).await?;
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        progress_bar.finish_with_message(format!(
            "{} {} Rows processed",
            "✅ [DONE]".green().bold(),
            &lines_count
        ));
        let _ = sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk).await?;
        chunk.clear();
    }

    Ok(())
}
