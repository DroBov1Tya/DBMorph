use colored::*;
use futures::{Stream, StreamExt};
use mongodb::{
    bson::{doc, Bson, Document},
    options::{Acknowledgment, IndexOptions, InsertManyOptions, WriteConcern},
    Collection, IndexModel,
};
use std::sync::Arc;
use std::{error::Error, pin::Pin};
use tokio::sync::Semaphore;
use tracing::error;

use super::mongo_utils;
use crate::utils;

pub async fn insert_stream_to_mongo<'a>(
    collection: &Collection<Document>,
    batch_size: Option<u32>,
    threads: u32,
    mut reader: Pin<
        Box<dyn Stream<Item = Result<Vec<String>, Box<dyn Error + Send + Sync>>> + Send + 'a>,
    >,
    total_rows: i64,
    headers_row: bool,
    custom_rows: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let preview_count = 5;
    let batch_size = batch_size.unwrap().try_into().unwrap_or(100);
    let semaphore = Arc::new(Semaphore::new(threads as usize));
    let mut preview_shown = false;
    let mut lines_count = 0usize;
    let mut preview_chunk: Vec<Vec<String>> = Vec::new();

    let progress_bar = utils::output_format::init_progress_bar(total_rows as u64).await?;

    let mut columns: Vec<String> = if headers_row {
        match StreamExt::next(&mut reader).await {
            Some(Ok(header)) => header,
            Some(Err(e)) => {
                error!("Failed to read header row: {}", e);
                println!(
                    "{}   Failed to read header row: {}",
                    "🚫 [CSV]".red().bold(),
                    e
                );
                return Err("Header parse error".into());
            }
            None => {
                error!("No rows in stream – possible empty or corrupted file");
                println!(
                    "{}   No rows found in input stream.",
                    "🚫 [CSV]".red().bold()
                );
                return Err("Empty stream".into());
            }
        }
    } else {
        vec![]
    };

    let mut chunk: Vec<Document> = Vec::with_capacity(batch_size);

    match custom_rows {
        Some(ref row) => {
            let vec: Vec<String> = row.split(' ').map(|s| s.trim().to_string()).collect();
            preview_chunk.push(vec.clone());
            columns = vec;
        }
        _ => {}
    };

    while let Some(row_res) = StreamExt::next(&mut reader).await {
        match row_res {
            Ok(row) => {
                if !preview_shown && preview_chunk.len() < preview_count {
                    preview_chunk.push(row.clone());
                    if preview_chunk.len() == preview_count {
                        utils::cli_prompt::process_and_pause(preview_chunk.clone()).await?;
                        preview_shown = true;
                    }
                }

                let mut doc = if headers_row {
                    let mut doc = Document::new();
                    for (col, val) in columns.iter().zip(row.iter()) {
                        doc.insert(col.to_uppercase(), Bson::String(val.clone().to_uppercase()));
                    }
                    doc
                } else {
                    let mut doc = Document::new();
                    for (i, val) in row.iter().enumerate() {
                        let key = format!("c{}", i + 1);
                        doc.insert(key.to_uppercase(), Bson::String(val.clone().to_uppercase()));
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
                eprintln!("{} Failed to read row: {}", "🚫 [ERROR]".red().bold(), e);
            }
        }
    }

    if !chunk.is_empty() {
        collection.insert_many(std::mem::take(&mut chunk)).await?;
    }

    chunk.clear();

    progress_bar.finish_with_message(format!(
        "{} {} Rows inserted into MongoDB",
        "✅ [DONE]".green().bold(),
        lines_count
    ));

    Ok(())
}
