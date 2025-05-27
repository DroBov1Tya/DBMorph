use colored::*;
use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};

use super::output_format;

pub async fn process_and_pause(
    preview: Vec<Vec<String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    /// Displays a preview of up to 5 records in CSV format,
    /// then repeatedly prompts the user to confirm continuation:
    /// accepts "y" or Enter to proceed, "n" to exit, and loops on invalid input.
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

pub async fn select_table(
    database_tables: &BTreeSet<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    /// Displays a numbered list of table names from the given set,
    /// prompts the user to select a table by entering its number,
    /// then returns the chosen table name or an error string if invalid.
    let mut index_table_map: HashMap<usize, String> = HashMap::new();

    println!(
        "{}",
        "____________________________________________________________"
            .yellow()
            .bold()
    );
    for (idx, name) in database_tables.iter().enumerate() {
        index_table_map.insert(idx, name.clone());
        println!("{}: {}", idx.to_string().yellow().bold(), name.italic());
    }
    println!(
        "{}",
        "____________________________________________________________"
            .yellow()
            .bold()
    );

    print!("{}", "\nChoose table number: ".green().bold());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let selected_index: usize = input.trim().parse::<usize>()?;

    match index_table_map.get(&selected_index) {
        Some(table_name) => {
            println!("\nChoosen table: {}\n", table_name.green().bold());
            Ok(table_name.clone())
        }
        None => {
            println!("{}", "Can't find table number {} in map".red());
            Ok("Err".to_string())
        }
    }
}
