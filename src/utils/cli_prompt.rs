use colored::*;
use std::io::{self, Write};

use super::output_format;

pub async fn process_and_pause(preview: Vec<Vec<String>>) -> Result<(), Box<dyn std::error::Error>> {
    let mut first_records: Vec<Vec<String>> = Vec::new();

    for record in preview.iter().take(5) {
        first_records.push(record.clone());
    }

    output_format::print_csv_table(&first_records).await;

    loop {
        println!(
            "{} {} [{}/{}] ({})",
            "⚠️",
            "Continue?".bold(),
            "Y".green().bold(),
            "n".red().bold(),
            "default: Enter".cyan().italic()
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim().to_lowercase().as_str() {
            "y" => {
                return Ok(());
            }
            "" => {
                return Ok(());
            }
            "n" => {
                eprintln!("{}   Execution terminated.", "🚫 [ERROR]".red().bold());
                std::process::exit(0);
            }
            _ => {
                eprintln!(
                    "{}   Invalid input, please try again.",
                    "🚫 [ERROR]".red().bold()
                );
            }
        }
    }
}
