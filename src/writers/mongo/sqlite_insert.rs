use colored::*;
use futures::{Stream, StreamExt};
use mongodb::{
    bson::{Bson, Document},
    Collection,
};
use std::{error::Error, pin::Pin};

use crate::utils;

pub async fn insert_stream_to_mongo<'a>(
    collection: &Collection<Document>,
    batch_size: Option<u32>,
    mut reader: Pin<
        Box<dyn Stream<Item = Result<Vec<String>, Box<dyn Error + Send + Sync>>> + Send + 'a>,
    >,
    total_rows: i64,
    headers_row: bool,
) -> Result<(), Box<dyn Error>> {
    let preview_count = 5;
    let batch_size = batch_size.unwrap().try_into().unwrap_or(100);
    let mut preview_shown = false;
    let mut lines_count = 0usize;
    let mut preview_chunk: Vec<Vec<String>> = Vec::new();

    let progress_bar = utils::output_format::init_progress_bar(total_rows as u64).await?;

    let columns: Vec<String> = if headers_row {
        match StreamExt::next(&mut reader).await {
            Some(Ok(header)) => header,
            Some(Err(e)) => {
                eprintln!(
                    "{} Failed to read header row: {}",
                    "🚫 [ERROR]".red().bold(),
                    e
                );
                return Err("Header parse error".into());
            }
            None => {
                eprintln!("{} No rows in stream", "🚫 [ERROR]".red().bold());
                return Err("Empty stream".into());
            }
        }
    } else {
        vec![]
    };

    let mut chunk: Vec<Document> = Vec::with_capacity(batch_size);

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

                let doc = if headers_row {
                    let mut doc = Document::new();
                    for (col, val) in columns.iter().zip(row.iter()) {
                        doc.insert(col, Bson::String(val.clone()));
                    }
                    doc
                } else {
                    let mut doc = Document::new();
                    for (i, val) in row.iter().enumerate() {
                        let key = format!("c{}", i + 1);
                        doc.insert(key, Bson::String(val.clone()));
                    }
                    doc
                };

                chunk.push(doc);
                lines_count += 1;

                if chunk.len() >= batch_size {
                    progress_bar.set_position(lines_count as u64);
                    collection.insert_many(std::mem::take(&mut chunk)).await?;
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

    progress_bar.finish_with_message(format!(
        "{} {} Rows inserted into MongoDB",
        "✅ [DONE]".green().bold(),
        lines_count
    ));

    Ok(())
}
