//! Separate, on-demand details window; native Windows tray process stays small.
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use g435_battery::history::BatteryHistory;
use std::time::Duration;

pub fn show() -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([350.0, 345.0])
            .with_resizable(false)
            .with_always_on_top(),
        ..Default::default()
    };
    // Convert framework's error to a normal owned error for main.
    eframe::run_native("G435 Battery Monitor", options, Box::new(|_| Ok(Box::new(DetailsCard))))
        .map_err(|error| std::io::Error::other(error.to_string()).into())
}

struct DetailsCard;

impl eframe::App for DetailsCard {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());
        ctx.request_repaint_after(Duration::from_secs(5));
        let history = BatteryHistory::load();
        let now = BatteryHistory::now();
        let accent = Color32::from_rgb(93, 210, 173);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("Logitech G435");
            ui.label(egui::RichText::new("LIGHTSPEED  /  USB receiver").weak());
            ui.add_space(12.0);
            if let Some(sample) = history.latest() {
                let stale = now.saturating_sub(sample.at) > 180;
                ui.label(egui::RichText::new(format!("~{}%", sample.percent)).size(36.0).strong().color(accent));
                ui.add(egui::ProgressBar::new(f32::from(sample.percent) / 100.0).fill(accent));
                ui.label(format!("{} mV  •  {}", sample.millivolts,
                    if stale { "Last recorded reading" } else { "Voltage-based estimate" }));
                if let Some(hours) = history.hours_remaining(now) {
                    ui.label(format!("Estimated remaining: {:.1} hours", hours));
                } else {
                    ui.label(egui::RichText::new("Remaining time: collecting discharge data").weak());
                }
            } else {
                ui.label(egui::RichText::new("--%").size(36.0));
                ui.label("No recorded reading yet. Turn the headset on.");
            }
            ui.add_space(12.0);
            ui.separator();
            ui.label(egui::RichText::new("BATTERY HISTORY  ·  LAST 12 HOURS").small().strong());
            let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 78.0), egui::Sense::hover());
            let plot = response.rect.shrink2(Vec2::new(4.0, 4.0));
            draw_chart(&history, now, plot, &painter, accent);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("12h ago").weak().small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new("Now").weak().small());
                });
            });
            ui.add_space(5.0);
            ui.label(egui::RichText::new("Charging status unavailable  •  Data stored locally").weak().small());
        });
    }
}

fn draw_chart(history: &BatteryHistory, now: u64, rect: Rect, painter: &egui::Painter, accent: Color32) {
    for fraction in [0.0, 0.5, 1.0] {
        let y = rect.bottom() - rect.height() * fraction;
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(65)));
    }
    let start = now.saturating_sub(12 * 3600);
    let mut previous: Option<(u64, Pos2)> = None;
    for sample in history.samples().iter().filter(|s| s.at >= start && s.at <= now) {
        let x = rect.left() + (sample.at - start) as f32 / (12 * 3600) as f32 * rect.width();
        let y = rect.bottom() - f32::from(sample.percent) / 100.0 * rect.height();
        let point = Pos2::new(x, y);
        if let Some((at, pos)) = previous {
            if sample.at - at <= 600 {
                painter.line_segment([pos, point], Stroke::new(2.0, accent));
            }
        }
        painter.circle_filled(point, 2.0, accent);
        previous = Some((sample.at, point));
    }
    if previous.is_none() {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Waiting for data",
            egui::FontId::proportional(12.0), Color32::GRAY);
    }
}
