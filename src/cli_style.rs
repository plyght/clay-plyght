use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

pub struct CliStyle;

impl CliStyle {
    pub fn success(text: &str) -> String {
        format!("{} {}", style("✓").green().bold(), style(text).white())
    }

    pub fn error(text: &str) -> String {
        format!("{} {}", style("✗").red().bold(), style(text).white())
    }

    pub fn warning(text: &str) -> String {
        format!("{} {}", style("!").yellow().bold(), style(text).white())
    }

    pub fn info(text: &str) -> String {
        format!("{} {}", style("i").blue().bold(), style(text).white())
    }

    pub fn arrow(text: &str) -> String {
        format!("{} {}", style("→").cyan(), style(text).white())
    }

    pub fn bullet(text: &str) -> String {
        format!("{} {}", style("•").dim(), style(text).white())
    }

    pub fn package_name(name: &str) -> String {
        style(name).white().bold().to_string()
    }

    pub fn version(version: &str) -> String {
        style(version).green().to_string()
    }

    pub fn dim_text(text: &str) -> String {
        style(text).dim().to_string()
    }

    pub fn cyan_text(text: &str) -> String {
        style(text).cyan().to_string()
    }

    pub fn create_spinner(message: &str) -> ProgressBar {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.set_message(message.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    pub fn create_progress_bar(total: u64) -> ProgressBar {
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} {bar:40.green/dim} {pos:>3}/{len:3} {msg}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏  "),
        );
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    }

    pub fn format_duration(duration: std::time::Duration) -> String {
        if duration.as_millis() < 1000 {
            format!("{}ms", duration.as_millis())
        } else if duration.as_secs() < 60 {
            format!("{:.1}s", duration.as_millis() as f64 / 1000.0)
        } else {
            format!("{}m {}s", duration.as_secs() / 60, duration.as_secs() % 60)
        }
    }

    pub fn format_size(bytes: u64) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
        let mut size = bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{} {}", size as u64, UNITS[unit_index])
        } else {
            format!("{:.1} {}", size, UNITS[unit_index])
        }
    }

    pub fn section_header(title: &str) -> String {
        style(title).blue().bold().to_string()
    }

    pub fn command_suggestion(command: &str) -> String {
        style(command).cyan().to_string()
    }

    pub fn highlight(text: &str) -> String {
        style(text).white().bold().to_string()
    }
}
