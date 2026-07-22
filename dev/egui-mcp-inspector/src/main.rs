fn main() -> eframe::Result {
    eframe::run_native(
        "Labello MCP Inspector",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1440.0, 1000.0]),
            ..Default::default()
        },
        Box::new(|creation_context| {
            creation_context.egui_ctx.enable_accesskit();
            Ok(Box::new(labello_ui::LabelloApp::default()))
        }),
    )
}
