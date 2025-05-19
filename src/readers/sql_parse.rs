use clap::builder::Str;
use colored::Colorize;
use std::collections::HashMap;
use std::error::Error;
use tokio::fs;

pub async fn parse_dump_simple(file: String) -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(file.as_str()).await?;

    let mut databases = Vec::new();
    let mut tables = Vec::new();

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with("--") || line.starts_with("#") || line.is_empty() {
            continue;
        }

        if line.to_uppercase().starts_with("CREATE DATABASE") {
            if let Some(name) = extract_name_from_create(line, "CREATE DATABASE") {
                databases.push(name);
            }
            continue;
        }

        if line.to_uppercase().starts_with("CREATE TABLE") {
            if let Some(name) = extract_name_from_create(line, "CREATE TABLE") {
                tables.push(name);
            }
            continue;
        }
    }

    println!(
        "{}   Databases found: {}",
        "✅ [DB]".green().bold(),
        databases.len()
    );
    for db in &databases {
        println!(" - {}", db);
    }

    println!(
        "{}   Tables discovered: {}",
        "✅ [DB]".green().bold(),
        tables.len()
    );
    for table in &tables {
        println!(" - {}", table);
    }

    let rows = extract_insert_values(&file, "btc_addresses");

    println!("{}   Rows found: {}", "✅ [DB]".green().bold(), rows.len());
    for row in rows.iter().take(3) {
        println!("{:?}", row);
    }

    Ok(())
}

fn extract_name_from_create(line: &str, keyword: &str) -> Option<String> {
    let after_keyword = line[keyword.len()..].trim();

    let after_keyword = if after_keyword.to_uppercase().starts_with("IF NOT EXISTS") {
        after_keyword["IF NOT EXISTS".len()..].trim()
    } else {
        after_keyword
    };

    if after_keyword.starts_with('`') {
        after_keyword.split('`').nth(1).map(|s| s.to_string())
    } else {
        after_keyword
            .split_whitespace()
            .next()
            .map(|s| s.trim_matches(';').to_string())
    }
}

pub fn extract_insert_values(sql: &str, table_name: &str) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    let mut collecting = false;
    let mut buffer = String::new();

    for line in sql.lines() {
        let trimmed = line.trim();

        if !collecting && trimmed.starts_with(&format!("INSERT INTO `{}`", table_name)) {
            collecting = true;
            if let Some(idx) = trimmed.find("VALUES") {
                buffer.push_str(&trimmed[idx + 6..]);
            }
            continue;
        }

        if collecting {
            buffer.push_str(trimmed);
            if trimmed.ends_with(';') {
                collecting = false;
                break;
            }
        }
    }

    // Удаляем завершающую точку с запятой
    let cleaned = buffer.trim_end_matches(';').trim();

    // Разбиваем по "),(" границе между записями
    for row in cleaned.split("),(") {
        let row_clean = row.trim_start_matches('(').trim_end_matches(')').trim();
        let values = row_clean
            .split(',')
            .map(|s| s.trim().trim_matches('\'').to_string())
            .collect::<Vec<_>>();
        result.push(values);
    }

    result
}
