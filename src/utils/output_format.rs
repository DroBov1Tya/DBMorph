use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use std::error::Error;
use tabled::builder::Builder;
use tokio::io::{self, AsyncWriteExt};

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
                                                            
Version: v0.3.3-dev
    "#
        .green(),
        "by github.com/DroBov1Tya".bright_cyan().italic(),
        "____________________________________________________________".green(),
    );
}

pub async fn print_csv_table(records: &[Vec<String>]) {
    // Prints a preview of the input CSV data showing up to 5 rows and 5 columns.
    // If no records are provided, displays a warning message.
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
    // Displays a progress status line showing the current processed line count
    // out of the total number of rows, updating the same console line asynchronously.
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
    // Prints a final message indicating the total number of rows processed,
    // marking the completion of the SQLite processing task.
    let status = format!(
        "{}    Rows processed: {}",
        "✅ [DONE]".green().bold(),
        &lines_count
    );

    print!("\r{}", status);
}

pub async fn init_progress_bar(total_rows: u64) -> Result<ProgressBar, Box<dyn Error>> {
    // Initializes and returns a styled progress bar for tracking progress over a given total number of rows.
    // The progress bar includes spinner, percentage, progress bar visualization, current position, total length,
    // estimated time remaining, and processing speed.
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
