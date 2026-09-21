use colored::*;
use std::io::{self, Write};
use tabled::builder::Builder;
use tabled::settings::Style;

const LABEL_WIDTH: usize = 13;

pub fn banner(version: &str) {
    let art = r#"
    ██████╗ ██████╗ ███╗   ███╗ ██████╗ ██████╗ ██████╗ ██╗  ██╗
    ██╔══██╗██╔══██╗████╗ ████║██╔═══██╗██╔══██╗██╔══██╗██║  ██║
    ██║  ██║██████╔╝██╔████╔██║██║   ██║██████╔╝██████╔╝███████║
    ██║  ██║██╔══██╗██║╚██╔╝██║██║   ██║██╔══██╗██╔═══╝ ██╔══██║
    ██████╔╝██████╔╝██║ ╚═╝ ██║╚██████╔╝██║  ██║██║     ██║  ██║
    ╚═════╝ ╚═════╝ ╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚═╝     ╚═╝  ╚═╝"#;

    println!("{}", art.cyan());
    println!(
        "    {} {}   {}\n",
        "data pipeline".bright_white().bold(),
        format!("v{version}").bright_black(),
        "· by DroBoV1tya".bright_black().italic()
    );
}

pub fn section(title: &str) {
    println!("\n  {}", title.to_uppercase().bright_white().bold());
    println!("  {}", "─".repeat(title.len().max(8)).bright_black());
}

pub fn field(label: &str, value: &str) {
    println!(
        "  {:<LABEL_WIDTH$} {}",
        label.bright_black(),
        value.bright_white()
    );
}

pub fn step(msg: &str) {
    println!("  {} {}", "▸".cyan().bold(), msg.bright_white());
}

pub fn ok(msg: &str) {
    println!("  {} {}", "✓".green().bold(), msg.bright_white());
}

pub fn warn(msg: &str) {
    println!("  {} {}", "!".yellow().bold(), msg.yellow());
}

pub fn error(msg: &str) {
    eprintln!("  {} {}", "✗".red().bold(), msg.red());
}

pub fn info(msg: &str) {
    println!("  {} {}", "·".bright_black(), msg.bright_black());
}

pub fn progress(label: &str, current: u64, total: Option<u64>) {
    let body = match total {
        Some(total) if total > 0 => {
            let pct = ((current as f64 / total as f64) * 100.0).min(100.0);
            format!(
                "{} {} / {}  {}",
                label.cyan(),
                fmt_int(current).bright_white().bold(),
                fmt_int(total).bright_black(),
                format!("{pct:>5.1}%").cyan()
            )
        }
        _ => format!(
            "{} {} rows",
            label.cyan(),
            fmt_int(current).bright_white().bold()
        ),
    };

    print!("\r  {} {body:<70}", "⣷".cyan());
    let _ = io::stdout().flush();
}

pub fn progress_done(label: &str, rows: u64, elapsed: &str) {
    print!("\r{:<90}\r", " ");
    println!(
        "  {} {} {} rows in {}",
        "✓".green().bold(),
        label.bright_white(),
        fmt_int(rows).bright_white().bold(),
        elapsed.cyan()
    );
}

pub fn preview_table(records: &[Vec<String>], max_rows: usize, max_cols: usize) {
    if records.is_empty() {
        warn("No records to display");
        return;
    }

    let rows: Vec<Vec<String>> = records
        .iter()
        .take(max_rows)
        .map(|row| {
            row.iter()
                .take(max_cols)
                .map(|cell| clip(cell, 32))
                .collect()
        })
        .collect();

    let mut builder = Builder::default();
    for row in rows {
        builder.push_record(row);
    }

    let mut table = builder.build();
    table.with(Style::rounded());

    println!(
        "\n  {} first {max_rows} rows × {max_cols} cols",
        "preview".bright_black()
    );
    for line in table.to_string().lines() {
        println!("  {line}");
    }
}

pub fn total_time(elapsed: &str) {
    println!(
        "\n  {} {} {}",
        "◆".cyan().bold(),
        "total".bright_black(),
        elapsed.bright_white().bold()
    );
}

fn clip(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let head: String = chars[..max.saturating_sub(1)].iter().collect();
        format!("{head}…")
    }
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}
