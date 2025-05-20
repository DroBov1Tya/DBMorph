use std::error::Error;
use std::collections::BTreeMap;
use colored::*;
use futures::stream::{self, Stream};
use futures::StreamExt;
use sqlx::{sqlite::SqliteQueryResult, SqliteConnection};
use tabled::grid::records::ExactRecords;

use super::sqlite_init;
use crate::utils::output_format::{sqlite_processing, sqlite_processing_finish};
use crate::utils::cli_prompt::process_and_pause;

pub async fn processing_json(
    conn: &mut SqliteConnection,
    table_name: &String,
    columns: Vec<String>,
    mut json_stream: impl Stream<Item = Result<BTreeMap<String, String>, String>> + Unpin,
    batch_size: Option<u32>,
    lines_count: u32,
) -> Result<(), Box<dyn Error>> {
    let mut chunk: Vec<Vec<String>> = Vec::new();
    let max_chunk_size = batch_size.unwrap_or(1000);
    let mut preview_shown = false;
    let preview_count = 5;
    let mut total_rows = 0;
        
    while let Some(result) = json_stream.next().await {
        match result {
            Ok(flat_map) => {
                let row: Vec<String> = flat_map.into_values().collect();
                chunk.push(row);
                total_rows += 1;

                if !preview_shown && chunk.len() == preview_count {
                    let preview = chunk[..preview_count].to_vec();
                    let _ = process_and_pause(preview).await;
                    preview_shown = true;
                }

                if chunk.len() as u32 >= max_chunk_size {
                    let _ = sqlite_processing(total_rows, lines_count.try_into().unwrap()).await;
                    let _ = sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk).await?;
                    chunk.clear();
                }
            }
            Err(err) => {
                eprintln!("{} Error: {}", "🚫 [ERROR]".red().bold(), err);
            }
        }
    }
    if !chunk.is_empty() {
        let _ = sqlite_processing_finish(total_rows).await;
        let _ = sqlite_init::insert_chunk_to_sqlite(conn, &table_name, &columns, &chunk).await?;
        chunk.clear();
    }
    Ok(())
}