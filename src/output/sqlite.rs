use std::error::Error;
use tracing::{info, warn, error};
use crate::{input, output, transform};

mod requests;

pub async fn sqlite_processing(
    input_file: String,
    output_file: String,
    table_name: String,
    drop_existing: bool,
    batch_size: Option<u32>,
    delimiter: Option<u8>,
) -> Result<(), Box<dyn Error>>{

    if drop_existing == true {
        let _delete_exists_table = requests::delete_exists_table(output_file.clone(), table_name.clone()).await;
    }

    let encoding = transform::encoding::detect_encoding(input_file.clone())?;
    
    info!("Detected input file charset encoding: {}", encoding);

    let max_collumns = input::csv_parse::check_max_collumns(input_file.clone(), delimiter.unwrap(), encoding.clone()).await;
    let create_sqlite_table = requests::create_fts5_table(output_file.clone(), table_name.clone(), max_collumns.unwrap()).await;

    let all_rows = input::csv_parse::csv_row_reader(input_file.clone(), delimiter.unwrap(), encoding).await.unwrap();

    let _start_process = requests::init_insert_process(output_file.clone(), table_name.clone(), create_sqlite_table.unwrap(), batch_size, all_rows).await;

    Ok(())
}