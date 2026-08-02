pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "my_ipkvm",
        options,
        Box::new(|cc| {
            crate::fonts::install(&cc.egui_ctx);
            Ok(Box::<DesktopApp>::default())
        }),
    )
}

#[derive(Default)]
struct DesktopApp;

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("my_ipkvm");
        });
    }
}
