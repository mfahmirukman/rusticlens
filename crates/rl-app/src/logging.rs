//! App-side tracing helpers. All user-facing diagnostics use target `rusticlens`
//! so `RUST_LOG=rusticlens=info` works as documented.

pub const TARGET: &str = "rusticlens";

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        tracing::info!(target: $crate::logging::TARGET, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        tracing::debug!(target: $crate::logging::TARGET, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        tracing::warn!(target: $crate::logging::TARGET, $($arg)*)
    };
}

pub fn init() {
    use tracing_subscriber::EnvFilter;

    // `rusticlens=info` matches our explicit target; `rl_app=info` matches default module paths.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("rusticlens=info,rl_app=info,rl_core=warn")
    });

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .init();

    log_info!(
        version = env!("CARGO_PKG_VERSION"),
        max_kb_per_log_tab = crate::ui::log_tabs::MAX_BYTES_PER_TAB / 1024,
        "rusticlens started — log buffer metrics appear when you open pod logs (RUST_LOG=rusticlens=info)"
    );
}
