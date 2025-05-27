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
    // Main async entry point that initializes logger and parses CLI arguments.
    // Based on the database type, it calls the appropriate async processing function
    // to import data from input file into the specified database.
    // Measures and prints total execution time after processing completes.
    let start = Instant::now();
    let _logger_init = utils::logger::logger();
    let arg = args::parse_args();

    utils::output_format::started_text().await;

    let input_file = arg.input_path;
    let output_file = arg.output_path;
    let table_name = arg.table_name;
    let column_count = arg.column_count;
    let encoding = arg.encoding;
    let delimiter = arg.delimiter;
    let remove_rows = arg.remove_rows;
    let batch_size = arg.batch_size;
    let drop_existing = arg.drop_existing;
    let headers_row = arg.headers_row;
    let database_type = arg.database_type;
    let database_url = arg.database_url;
    let database_user = arg.database_user;
    let database_pass = arg.database_pass;
    let input_file_type = arg.input_file_type;
    let threads = arg.threads;

    // let semaphore = Arc::new(Semaphore::new(threads.try_into().unwrap()));

    match database_type.as_str() {
        "sqlite" => {
            let _sqlite_processing = writers::sqlite::sqlite_processing(
                input_file,
                output_file,
                table_name,
                column_count,
                encoding,
                delimiter,
                remove_rows,
                drop_existing,
                batch_size,
                input_file_type,
            )
            .await;
        }
        "clickhouse" => {}
        "mysql" => {}
        "postgresql" => {}
        "mongodb" => {
            let _mongodb_processing = writers::mongo::mongodb_processing(
                input_file,
                output_file,
                table_name,
                column_count,
                encoding,
                delimiter,
                remove_rows,
                drop_existing,
                headers_row,
                database_url,
                database_user,
                database_pass,
                batch_size,
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
