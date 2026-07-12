mod app;
mod backend;
mod ui;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use eframe::egui;
use tracing_subscriber::EnvFilter;

use crate::ui::theme::Theme;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let backend = backend::spawn_backend();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("rusticlens"),
        ..Default::default()
    };

    eframe::run_native(
        "rusticlens",
        native_options,
        Box::new(|cc| {
            Theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::RusticlensApp::new(backend)))
        }),
    )
}
