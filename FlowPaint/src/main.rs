//! FlowPaint — a CFD paint program. Draw geometry, place fans and
//! drains, and watch real fluid dynamics computed on your GPU.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod geometry;
mod sim;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("FlowPaint — CFD you can finger paint"),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "FlowPaint",
        options,
        Box::new(|cc| Ok(Box::new(app::FlowPaintApp::new(cc)))),
    )
}
