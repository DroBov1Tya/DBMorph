use tracing_subscriber::FmtSubscriber;

pub fn logger() {
    FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .init();
}
