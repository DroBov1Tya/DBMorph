use colored::*;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;
use tabled::builder::Builder;
use tabled::settings::Style;

use crate::app::utils::progress::Progress;

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

pub fn progress(label: &str, rows: u64, progress: &Progress) {
    let rate = format!("{}/s", fmt_int(progress.rows_per_sec(rows)));
    let body = match progress.fraction(rows) {
        Some(fraction) => {
            let eta = progress
                .eta(rows)
                .map(|eta| format!("ETA {}", fmt_duration(eta)))
                .unwrap_or_default();
            format!(
                "{} {} rows  {}  {}  {}",
                label.cyan(),
                fmt_int(rows).bright_white().bold(),
                format!("{:>5.1}%", fraction * 100.0).cyan(),
                rate.bright_black(),
                eta.bright_black()
            )
        }
        None => format!(
            "{} {} rows  {}",
            label.cyan(),
            fmt_int(rows).bright_white().bold(),
            rate.bright_black()
        ),
    };

    // Clear the rest of the line: the colored body has escape codes, so
    // padding by width cannot hide a longer previous line.
    let clear = if io::stdout().is_terminal() {
        "\x1b[K"
    } else {
        ""
    };
    print!("\r  {} {body}{clear}", "⣷".cyan());
    let _ = io::stdout().flush();
}

fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
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
