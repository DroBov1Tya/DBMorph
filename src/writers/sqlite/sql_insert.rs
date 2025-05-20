use std::error::Error;
use sqlx::{SqliteConnection};
use colored::*;
use futures::pin_mut;
use futures::StreamExt;

use crate::readers::sql_parse;
use crate::utils::cli_prompt::process_and_pause;
use crate::utils::output_format::{sqlite_processing_finish, sqlite_processing};
use crate::writers::sqlite::sqlite_init;

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

    while let Some(row) = stream.next().await {
        chunk.push(row);
        lines_count += 1;

        if !preview_shown && chunk.len() == preview_count {
            let preview = chunk[..preview_count].to_vec();
            let _ = process_and_pause(preview).await;
            preview_shown = true;
        }

        if chunk.len() as u32 >= max_chunk_size {
            let _ = sqlite_processing(lines_count, total_rows.try_into().unwrap()).await;
            let _ =
                sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk)
                    .await?;
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        let _ = sqlite_processing_finish(lines_count).await;
        let _ = sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk).await?;
        chunk.clear();
    }

    Ok(())
}
