//! Terminal UI utilities for mockpit CLI
//!
//! Provides formatting helpers for CLI output. This module replaces the
//! bdg-ui crate with a self-contained implementation using colored and indicatif.

use colored::Colorize;
use comfy_table::{Cell, CellAlignment, Color, Table, presets};
use indicatif::{ProgressBar, ProgressStyle};
use std::fmt::Display;
use std::time::Duration;

// --- Status Messages ---

pub fn success(msg: &str) -> String {
    format!("{} {}", "✓".green().bold(), msg.bold())
}

pub fn error(msg: &str) -> String {
    format!("{} {}", "✗".red().bold(), msg.bold())
}

pub fn warning(msg: &str) -> String {
    format!("{} {}", "⚠".yellow().bold(), msg.bold())
}

pub fn info(msg: &str) -> String {
    format!("{} {}", "ℹ".blue(), msg.cyan())
}

pub fn action(msg: &str) -> String {
    format!("{} {}", "->".cyan(), msg.cyan())
}

// --- Formatting Utilities ---

pub fn header(msg: &str) -> String {
    format!("{} {}", "▸".cyan(), msg.cyan().bold())
}

pub fn kv(key: &str, value: &str) -> String {
    format!("  {} {}", format!("{key}:").dimmed(), value)
}

pub fn list_item(msg: &str) -> String {
    format!("  {} {msg}", "●".cyan())
}

pub fn sub_item(msg: &str) -> String {
    format!("    {} {msg}", "→".dimmed())
}

pub fn code(msg: &str) -> String {
    msg.green().bold().to_string()
}

pub fn path(p: &str) -> String {
    p.cyan().italic().to_string()
}

pub fn number(n: impl Display) -> String {
    n.to_string().yellow().bold().to_string()
}

pub fn emphasis(msg: &str) -> String {
    msg.bold().to_string()
}

pub fn dim(msg: &str) -> String {
    msg.dimmed().to_string()
}

pub fn step(step_num: usize, total: usize, msg: &str) -> String {
    format!("[{step_num}/{total}] {msg}")
}

// --- Dividers ---

pub fn divider() {
    println!("{}", "─".repeat(80).dimmed());
}

// --- Progress ---

pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    if let Ok(style) = ProgressStyle::with_template(concat!("{spinner:.cyan}", " {msg}")) {
        pb.set_style(style.tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]));
    }
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Extension trait for ProgressBar to add finish_success etc.
pub trait SpinnerExt {
    fn finish_success(&self, msg: &str);
    fn finish_warning(&self, msg: &str);
    fn finish_error(&self, msg: &str);
}

impl SpinnerExt for ProgressBar {
    fn finish_success(&self, msg: &str) {
        self.finish_with_message(success(msg));
    }
    fn finish_warning(&self, msg: &str) {
        self.finish_with_message(warning(msg));
    }
    fn finish_error(&self, msg: &str) {
        self.finish_with_message(error(msg));
    }
}

// --- Tables ---

pub fn table() -> Table {
    let mut t = Table::new();
    t.load_preset(presets::UTF8_FULL);
    t
}

pub fn table_header(text: &str) -> Cell {
    Cell::new(text)
        .fg(Color::Cyan)
        .set_alignment(CellAlignment::Left)
}

pub fn table_number_cell(n: impl Display) -> Cell {
    Cell::new(n)
        .fg(Color::Yellow)
        .set_alignment(CellAlignment::Right)
}

pub fn table_emphasis_cell(text: &str) -> Cell {
    Cell::new(text).set_alignment(CellAlignment::Left)
}

pub fn table_dim_cell(text: &str) -> Cell {
    Cell::new(text)
        .fg(Color::DarkGrey)
        .set_alignment(CellAlignment::Left)
}

// --- Formatting ---

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        let whole = bytes / GB;
        let frac = (bytes % GB) * 10 / GB;
        format!("{whole}.{frac} GB")
    } else if bytes >= MB {
        let whole = bytes / MB;
        let frac = (bytes % MB) * 10 / MB;
        format!("{whole}.{frac} MB")
    } else if bytes >= KB {
        let whole = bytes / KB;
        let frac = (bytes % KB) * 10 / KB;
        format!("{whole}.{frac} KB")
    } else {
        format!("{bytes} B")
    }
}

// --- Preview ---

pub fn preview_box(title: &str, content: &str) {
    println!("\n{}", format!("┌─ {title} ").cyan());
    for line in content.lines() {
        println!("{} {line}", "│".cyan());
    }
    println!("{}", "└─".cyan());
}
