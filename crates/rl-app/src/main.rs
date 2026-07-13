mod app;
mod backend;
mod logging;
mod ui;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use eframe::egui;

use crate::ui::theme::Theme;

fn main() -> eframe::Result<()> {
    logging::init();
    rl_core::ensure_plugins_dir();
    rl_core::write_example_manifest_if_missing();

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
