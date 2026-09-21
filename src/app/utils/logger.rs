use tracing_subscriber::FmtSubscriber;

pub fn logger() {
    let _ = FmtSubscriber::builder()
        .with_max_level(tracing::Level::WARN)
        .try_init();
}
