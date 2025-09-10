use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new(
        "debug,hyper_util=warn,reqwest=warn,rustls=warn",
    ));
    let fmt_layer = tracing_subscriber::fmt::layer();

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .init();
}
fn main() {
    init_tracing();

    deadlock_api_ingest_lib::run()
}
