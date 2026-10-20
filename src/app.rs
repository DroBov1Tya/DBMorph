use std::time::Instant;

use anyhow::Result;

pub mod doctor;
pub mod readers;
pub mod transform;
pub mod utils;
pub mod writers;

use crate::app::utils::ui;
use crate::args;
use crate::config;

pub async fn run() -> Result<u64> {
    utils::logger::logger();
    if std::env::args_os().len() == 1 {
        return crate::tui::run().await;
    }
    run_pipeline(args::parse_args()).await
}

pub async fn run_pipeline(mut args: crate::args::AppArgs) -> Result<u64> {
    let start = Instant::now();

    ui::banner(config::APP_VERSION);

    ui::section("pipeline");
    ui::field("input", &args.input_path);
    ui::field("output", &args.output_path);
    ui::field("source", &args.input_file_type);
    ui::field("target", &args.database_type);

    if args.doctor {
        if !doctor::applies(&args) {
            anyhow::bail!("--doctor is available only for csv/txt input");
        }
        let report = doctor::examine(&args)?;
        let applied = if args.no_auto_fix {
            Vec::new()
        } else {
            report.apply_safe(&mut args)
        };
        report.print(&applied);
        let suggested = report.suggested(&applied);
        let mut command_fixes = applied;
        command_fixes.extend(suggested);
        if !command_fixes.is_empty() {
            ui::field("suggested command", &doctor::command_with(&command_fixes));
        }
        return Ok(0);
    }

    if args.database_type == "parquet" {
        ui::field("codec", &format!("{:?}", args.compression).to_lowercase());
    }
    if args.database_type == "sqlite" {
        ui::field("mode", &format!("{:?}", args.sqlite_mode).to_lowercase());
    }

    let skipped = match args.database_type.as_str() {
        "sqlite" => writers::sqlite::sqlite_processing(&args).await?,
        "parquet" => writers::parquet::parquet_processing(&args).await?,
        "csv" => writers::csv::csv_processing(&args).await?,
        other => {
            ui::warn(&format!("Unknown target database: {other}"));
            0
        }
    };

    ui::total_time(&format!("{:.2?}", start.elapsed()));
    Ok(skipped)
}
