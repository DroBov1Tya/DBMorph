use std::process::ExitCode;

mod app;
mod args;
mod config;

#[tokio::main]
async fn main() -> ExitCode {
    match app::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            app::utils::ui::error(&format!("{err:#}"));
            ExitCode::FAILURE
        }
    }
}
