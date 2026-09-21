use std::time::Instant;

use anyhow::Result;

pub mod readers;
pub mod transform;
pub mod utils;
pub mod writers;

use crate::app::utils::ui;
use crate::args;
use crate::config;

pub async fn run() -> Result<()> {
    let start = Instant::now();
    utils::logger::logger();

    let args = args::parse_args();

    ui::banner(config::APP_VERSION);

    ui::section("pipeline");
    ui::field("input", &args.input_path);
    ui::field("output", &args.output_path);
    ui::field("source", &args.input_file_type);
    ui::field("target", &args.database_type);
    if args.database_type == "parquet" {
        ui::field("codec", &format!("{:?}", args.compression).to_lowercase());
    }

    match args.database_type.as_str() {
        "sqlite" => writers::sqlite::sqlite_processing(&args).await?,
        "parquet" => writers::parquet::parquet_processing(&args).await?,
        "csv" => writers::csv::csv_processing(&args).await?,
        other => ui::warn(&format!("Unknown target database: {other}")),
    }

    ui::total_time(&format!("{:.2?}", start.elapsed()));
    Ok(())
}
