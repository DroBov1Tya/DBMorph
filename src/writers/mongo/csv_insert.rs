use colored::*;
use csv::StringRecord;
use mongodb::{
    bson::{doc, Bson, Document},
    Collection,
};
use std::error::Error;
use tracing::{error, info, warn};

use crate::utils::cli_prompt::process_and_pause;
use crate::utils::output_format;

pub async fn init_insert_to_mongo(
    collection: &Collection<Document>,
    batch_size: Option<u32>,
    mut all_rows: impl Iterator<Item = Result<StringRecord, Box<dyn std::error::Error>>>,
    total_rows: usize,
    headers_row: bool,
) -> Result<(), Box<dyn Error>> {
    // Reads CSV rows, shows a preview of first few rows with pause for confirmation
    // Converts rows into MongoDB documents, using headers as field names if available
    // Inserts documents in batches into MongoDB collection while updating progress bar
    // Handles errors reading CSV rows and finishes with a completion message
    let preview_count = 5;
    let batch_size = batch_size.unwrap_or(100) as usize;
    let mut preview_shown = false;
    let mut lines_count = 0;
    let mut preview_chunk: Vec<Vec<String>> = Vec::new();
    let progress_bar = output_format::init_progress_bar(total_rows as u64).await?;

    let columns: Vec<String> = if headers_row {
        match all_rows.next() {
            Some(Ok(header_record)) => header_record.iter().map(|s| s.to_string()).collect(),
            _ => {
                error!("Не удалось считать заголовки из первой строки CSV.");
                return Err("Header parse error".into());
            }
        }
    } else {
        vec![]
    };

    let mut chunk: Vec<Document> = Vec::with_capacity(batch_size);

    for row_result in all_rows {
        match row_result {
            Ok(record) => {
                let row_fields: Vec<String> = record.iter().map(|v| v.to_string()).collect();

                if !preview_shown && preview_chunk.len() < preview_count {
                    preview_chunk.push(row_fields.clone());
                    if preview_chunk.len() == preview_count {
                        let _ = process_and_pause(preview_chunk.clone()).await;
                        preview_shown = true;
                    }
                }

                let doc = if headers_row {
                    let mut doc = Document::new();
                    for (col, val) in columns.iter().zip(record.iter()) {
                        doc.insert(col.clone(), Bson::String(val.to_string()));
                    }
                    doc
                } else {
                    let mut doc = Document::new();
                    for (i, val) in record.iter().enumerate() {
                        let key = format!("c{}", i + 1);
                        doc.insert(key, Bson::String(val.to_string()));
                    }
                    doc
                };

                chunk.push(doc);
                lines_count += 1;

                if chunk.len() >= batch_size {
                    progress_bar.set_position(lines_count as u64);
                    let mut docs_to_insert = std::mem::take(&mut chunk);
                    let result = collection
                        .insert_many(std::mem::take(&mut docs_to_insert))
                        .await;

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
        let mut docs_to_insert = std::mem::take(&mut chunk);
        let result = collection
            .insert_many(std::mem::take(&mut docs_to_insert))
            .await;

        progress_bar.finish_with_message(format!(
            "{} {} Rows inserted into MongoDB",
            "✅ [DONE]".green().bold(),
            lines_count
        ));
    }

    Ok(())
}
