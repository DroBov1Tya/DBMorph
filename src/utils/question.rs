use std::io::{self, Write};
use csv::{StringRecord};
use tabled::grid::records::ExactRecords;

use super::printed;

pub fn process_and_pause(preview: Vec<Vec<String>>) -> Result<(), Box<dyn std::error::Error>> {
    let mut first_records: Vec<Vec<String>> = Vec::new();
    
    for record in preview.iter().take(5) {
        first_records.push(record.clone());
    }

    printed::print_csv_table(&first_records);

    loop {
        print!("Хотите продолжить? (Y/y для продолжения, N/n для выхода): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        match input.trim().to_lowercase().as_str() {
            "y" => {
                return Ok(());
            },
            "" => {
                return Ok(());
            },
            "n" => {
                eprintln!("Завершаем выполнение программы.");
                std::process::exit(0);
            },
            _ => {
                eprintln!("Неверный ввод, попробуйте снова.");
            }
        }
    }
}