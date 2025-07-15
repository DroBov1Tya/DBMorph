use colored::*;
use csv::StringRecord;
use futures::future::{join, join_all};
use mongodb::{
    bson::{doc, Bson, Document},
    options::{IndexOptions, InsertManyOptions, WriteConcern},
    Collection, IndexModel,
};
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use super::mongo_utils;
use crate::utils::cli_prompt::process_and_pause;
use crate::utils::output_format;

pub async fn init_insert_to_mongo(
    collection: Collection<Document>,
    batch_size: Option<u32>,
    threads: u32,
    mut all_rows: impl Iterator<Item = Result<StringRecord, Box<dyn std::error::Error>>>,
    total_rows: usize,
    headers_row: bool,
    custom_rows: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // Reads CSV rows, shows a preview of first few rows with pause for confirmation
    // Converts rows into MongoDB documents, using headers as field names if available
    // Inserts documents in batches into MongoDB collection while updating progress bar
    // Handles errors reading CSV rows and finishes with a completion message
    let preview_count = 5;
    let batch_size = batch_size.unwrap_or(100) as usize;
    let semaphore = Arc::new(Semaphore::new(threads as usize));
    let progress_bar = output_format::init_progress_bar(total_rows as u64).await?;
    let mut preview_shown = false;
    let mut lines_count = 0;
    let mut preview_chunk: Vec<Vec<String>> = Vec::new();

    let mut columns: Vec<String> = if headers_row {
        match all_rows.next() {
            Some(Ok(header_record)) => header_record.iter().map(|s| s.to_string()).collect(),
            _ => {
                error!(
                    "{}   Failed to parse headers from CSV – aborting.",
                    "🚫 [ERROR]".red().bold()
                );
                return Err("Header parse error".into());
            }
        }
    } else {
        vec![]
    };

    match custom_rows {
        Some(ref row) => {
            let vec: Vec<String> = row.split(' ').map(|s| s.trim().to_string()).collect();
            preview_chunk.push(vec.clone());
            columns = vec;
        }
        _ => {}
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

                let mut doc = if headers_row {
                    let mut doc = Document::new();
                    for (col, val) in columns.iter().zip(record.iter()) {
                        doc.insert(
                            col.clone().to_uppercase(),
                            Bson::String(val.to_string().to_uppercase()),
                        );
                    }
                    doc
                } else {
                    let mut doc = Document::new();
                    for (i, val) in record.iter().enumerate() {
                        let key = format!("c{}", i + 1);
                        doc.insert(
                            key.to_uppercase(),
                            Bson::String(val.to_string().to_uppercase()),
                        );
                    }
                    doc
                };

                let flat_str = mongo_utils::flatten_bson(&doc).await;
                let _ = doc.insert("_flat", Bson::String(flat_str));

                chunk.push(doc);
                lines_count += 1;

                if chunk.len() >= batch_size {
                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    progress_bar.set_position(lines_count as u64);
                    let collection = collection.clone();
                    let mut docs_to_insert = std::mem::take(&mut chunk);

                    let _result = collection
                        .clone()
                        .insert_many(std::mem::take(&mut docs_to_insert))
                        .await;

                    // tokio::spawn(async move {
                    //     let _result = collection
                    //         .clone()
                    //         .insert_many(std::mem::take(&mut docs_to_insert))
                    //         .await;
                    // });

                    drop(permit);
                    chunk.clear();
                }
            }
            Err(e) => {
                error!(
                    "{}   Failed to read row from CSV: {}",
                    "🚫 [ERROR]".red().bold(),
                    e
                );
            }
        }
    }

    if !chunk.is_empty() {
        let mut docs_to_insert = std::mem::take(&mut chunk);
        let collection = collection.clone();

        let _result = collection
            .clone()
            .insert_many(std::mem::take(&mut docs_to_insert))
            .await;
    }

    chunk.clear();

    progress_bar.set_position(lines_count as u64);
    progress_bar.finish_with_message(format!(
        "{} {} Rows inserted into MongoDB",
        "✅ [DONE]".green().bold(),
        lines_count
    ));

    Ok(())
}
