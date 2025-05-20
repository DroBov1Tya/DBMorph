use colored::*;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::Instant;
use tracing::{info, warn};

mod args;
mod config;
mod readers;
mod transform;
mod utils;
mod writers;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let start = Instant::now();
    let _logger_init = utils::logger::logger();
    let args = args::parse_args();

    utils::output_format::started_text().await;

    let input_file = args.input_path;
    let output_file = args.output_path;
    let table_name = args.table_name;
    let column_count = args.column_count;
    let encoding = args.encoding;
    let delimiter = args.delimiter;
    let batch_size = args.batch_size;
    let drop_existing = args.drop_existing;
    let database_type = args.database_type;
    let input_file_type = args.input_file_type;
    let threads = args.threads;

    // let semaphore = Arc::new(Semaphore::new(threads.try_into().unwrap()));

    match database_type.as_str() {
        "sqlite" => {
            let _sqlite_processing = writers::sqlite::sqlite_processing(
                input_file,
                output_file,
                table_name,
                column_count,
                encoding,
                drop_existing,
                batch_size,
                delimiter,
                input_file_type,
            )
            .await;
        }
        _ => {}
    }

    let duration = start.elapsed();
    println!(
        "{}  {} {}",
        "✅ [INFO]".cyan().bold(),
        "Total execution time:".bright_white(),
        format!("{:?}", duration).yellow()
    );
    Ok(())
}
