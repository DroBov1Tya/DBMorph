use std::process::ExitCode;

mod app;
mod args;
mod config;
mod tui;

#[tokio::main]
async fn main() -> ExitCode {
    let tui_mode = std::env::args_os().len() == 1;
    match app::run().await {
        Ok(0) => ExitCode::SUCCESS,
        Ok(skipped) => {
            if !tui_mode {
                app::utils::ui::warn(&format!(
                    "completed with {skipped} repaired row(s) or value(s) - see warnings above"
                ));
            }
            ExitCode::from(2)
        }
        Err(err) => {
            if !tui_mode || !tui::error_was_shown() {
                app::utils::ui::error(&format!("{err:#}"));
            }
            ExitCode::FAILURE
        }
    }
}
