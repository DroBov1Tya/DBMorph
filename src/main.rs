use std::error::Error;
use std::sync::Arc;
use tokio::time::Instant;
use tokio::sync::Semaphore;
use tracing::{info, warn};

mod input;
mod output;
mod transform;
mod utils;
mod config;
mod args;


#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let start = Instant::now();
    let _logger_init = utils::logging::logger();
    let args = args::parse_args();
    
    info!("[+] Programm started ...");

    let input_file = args.input_path;
    let output_file = args.output_path;
    let table_name = args.table_name;
    let column_count = args.column_count;
    let encoding = args.encoding;
    let delimiter = args.delimiter;
    let batch_size = args.batch_size;
    let drop_existing = args.drop_existing;
    let database_type = args.database_type;
    let threads = args.threads;

    match database_type.as_str() {
        "sqlite" => {
            let _sqlite_processing = output::sqlite::sqlite_processing(input_file, output_file, table_name, drop_existing, batch_size, delimiter).await;
        },
        _ => {
        }
    }
    // let semaphore = Arc::new(Semaphore::new(threads.try_into().unwrap()));
    
    let duration = start.elapsed();
    info!("[+] Total execution time: {:?}", duration);
    Ok(())
}