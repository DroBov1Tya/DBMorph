use std::error::Error;
use std::sync::Arc;
use tokio::time::Instant;
use tokio::sync::Semaphore;
use tracing::info;

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
    let threads = args.threads;

    if drop_existing == true {
        let _delete_exists_table = output::sqlite::delete_exists_table(output_file.clone(), table_name.clone());
    }

    let sqlite_columns = output::sqlite::create_fts5_table(output_file.clone(), table_name.clone(), column_count);
    let encoding = transform::encoding::detect_encoding(input_file.clone())?;

    info!("Detected input file charset encoding: {}", encoding);

    let all_rows = input::csv_parse::csv_row_reader(input_file.clone(), delimiter.unwrap(), encoding).unwrap();

    let _start_process = output::sqlite::init_insert_process(output_file.clone(), table_name.clone(), sqlite_columns.unwrap(), batch_size, all_rows);
    // let semaphore = Arc::new(Semaphore::new(threads.try_into().unwrap()));
    
    let duration = start.elapsed();
    info!("[+] Total execution time: {:?}", duration);
    Ok(())
}