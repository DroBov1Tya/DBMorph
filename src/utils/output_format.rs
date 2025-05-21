use std::error::Error;
use tokio::io::{self, AsyncWriteExt};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use tabled::builder::Builder;

pub async fn started_text() {
    println!(
        "{}\n{}\n{}\n",
        r#"
██████╗ ██████╗ ███╗   ███╗ ██████╗ ██████╗ ██████╗ ██╗  ██╗
██╔══██╗██╔══██╗████╗ ████║██╔═══██╗██╔══██╗██╔══██╗██║  ██║
██║  ██║██████╔╝██╔████╔██║██║   ██║██████╔╝██████╔╝███████║
██║  ██║██╔══██╗██║╚██╔╝██║██║   ██║██╔══██╗██╔═══╝ ██╔══██║
██████╔╝██████╔╝██║ ╚═╝ ██║╚██████╔╝██║  ██║██║     ██║  ██║
╚═════╝ ╚═════╝ ╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚═╝     ╚═╝  ╚═╝
                                                            
Version: v0.2.2-dev
    "#
        .green(),
        "by github.com/DroBov1Tya".bright_cyan().italic(),
        "____________________________________________________________".green(),
    );
}

pub async fn print_csv_table(records: &[Vec<String>]) {
    if records.is_empty() {
        println!("{}    No records to display.", "⚠️ [WARN]".yellow().bold());
        return;
    }
    let max_cols = 5;
    let max_rows = 5;

    let truncated_records: Vec<Vec<String>> = records
        .iter()
        .take(max_rows)
        .map(|row| row.iter().take(max_cols).cloned().collect())
        .collect();

    let mut builder = Builder::default();

    for row in truncated_records {
        builder.push_record(row);
    }

    let table = builder.build();
    println!(
        "{} Previewing the first 5 rows and 5 columns of the input dataset:",
        "ℹ️ [INFO]".cyan().bold()
    );
    println!("{}", table);
}

pub async fn sqlite_processing(lines_count: i32, total_rows: usize) {
    let status = format!(
        "\r{} {} / {}",
        "🔄 [PROCESSING]".bold().cyan(),
        lines_count.to_string().bold().yellow(),
        total_rows.to_string().bold().bright_black()
    );

    print!("{:<80}", status);
    io::stdout().flush().await.unwrap();
}

pub async fn sqlite_processing_finish(lines_count: i32) {
    let status = format!(
        "{}    Rows processed: {}",
        "✅ [DONE]".green().bold(),
        &lines_count
    );

    print!("\r{}", status);
}

pub async fn init_progress_bar(total_rows: u64) -> Result<ProgressBar, Box<dyn Error>> {
    let progress_bar = ProgressBar::new(total_rows);

    progress_bar.set_style(
        ProgressStyle::with_template(
            "{spinner} ⚡ {percent}% [{bar:35.blue/black}] ⚙️ {pos}/{len} | ⚡ ETA: {eta_precise} • {per_sec}" 
        )
        .unwrap()
        .progress_chars("█░ "),
    );

    Ok(progress_bar)
}