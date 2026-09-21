use std::io::{self, Write};

use anyhow::Result;
use colored::*;

use crate::app::utils::ui;
use crate::config;

pub async fn process_and_pause(preview: Vec<Vec<String>>) -> Result<()> {
    ui::preview_table(&preview, config::PREVIEW_ROWS, config::PREVIEW_COLS);

    loop {
        print!(
            "\n  {} continue? {} / {} (Enter = yes): ",
            "?".cyan().bold(),
            "y".green().bold(),
            "n".red().bold()
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim().to_lowercase().as_str() {
            "y" | "" => return Ok(()),
            "n" => {
                ui::warn("Execution terminated by user");
                std::process::exit(0);
            }
            _ => ui::error("Invalid input, try again"),
        }
    }
}
