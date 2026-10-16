use std::process::ExitCode;

mod app;
mod args;
mod config;

#[tokio::main]
async fn main() -> ExitCode {
    match app::run().await {
        Ok(0) => ExitCode::SUCCESS,
        Ok(skipped) => {
            app::utils::ui::warn(&format!(
                "completed with {skipped} malformed row(s) - see warnings above"
            ));
            ExitCode::from(2)
        }
        Err(err) => {
            app::utils::ui::error(&format!("{err:#}"));
            ExitCode::FAILURE
        }
    }
}
