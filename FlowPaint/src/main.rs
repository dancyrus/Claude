//! FlowPaint — a CFD paint program. Draw geometry, place fans and
//! drains, and watch real fluid dynamics computed on your GPU.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod generators;
mod geometry;
mod geomops;
mod model;
mod sim;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("FlowPaint V2 — CFD you can finger paint"),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            power_preference: wgpu::PowerPreference::HighPerformance,
            // Ask for everything the adapter can give: the default 128 MB
            // storage-binding limit is too small for large simulation
            // domains with off-screen margins.
            device_descriptor: std::sync::Arc::new(|adapter| wgpu::DeviceDescriptor {
                label: Some("flowpaint"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::default(),
            }),
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
