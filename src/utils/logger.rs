use tracing_subscriber::FmtSubscriber;

pub fn logger() {
    /// Initializes the global logger with INFO level filtering using `tracing` crate.
    FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .init();
}
