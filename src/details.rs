//! Independent details and settings window; the monitor remains lightweight.
use crate::runtime;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use wireless_battery_monitor::{
    battery::DeviceKind, history::BatteryHistory,
    preferences::{LiveSnapshot, Settings},
};
use std::time::Duration;

pub fn show(open_settings: bool) -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([365.0f32, 440.0])
            .with_min_inner_size([350.0f32, 400.0])
            .with_icon(egui::IconData {
                rgba: include_bytes!(concat!(env!("OUT_DIR"), "/app-icon.rgba")).to_vec(),
                width: 64, height: 64,
            })
            .with_always_on_top(),
        ..Default::default()
    };
    let mut settings = Settings::load();
    if let Ok(enabled) = runtime::startup_enabled() { settings.start_with_windows = enabled; }
    let app = DetailsCard {
        selected: DeviceKind::Headset, settings_page: open_settings,
        original: settings.clone(), settings, message: None,
    };
    eframe::run_native("Wireless Battery Monitor — Details & Settings", options, Box::new(|_| Ok(Box::new(app))))
        .map_err(|error| std::io::Error::other(error.to_string()).into())
}

struct DetailsCard {
    selected: DeviceKind,
    settings_page: bool,
    original: Settings,
    settings: Settings,
    message: Option<(bool, String)>,
}

impl DetailsCard {
    fn battery_page(&self, ui: &mut egui::Ui) {
        let now = BatteryHistory::now();
        let accent = Color32::from_rgb(93, 210, 173);
        let history = BatteryHistory::load(self.selected);
        let reading = LiveSnapshot::load().reading(self.selected, now);
        ui.heading(self.selected.name());
        ui.label(egui::RichText::new(match self.selected {
            DeviceKind::Headset => "LIGHTSPEED  /  USB receiver",
            DeviceKind::Mouse => "2.4 GHz  /  USB receiver or cable",
        }).weak());
        ui.add_space(12.0);
        if let Some(level) = reading.level {
            let color = if reading.stale { Color32::GRAY }
                else if reading.charging == Some(true) || reading.confirmed_full() { accent }
                else if level <= Settings::load().low_threshold { Color32::LIGHT_RED } else { accent };
            let estimated = reading.estimated || reading.stale;
            ui.label(egui::RichText::new(format!("{}{}%", if estimated { "~" } else { "" }, level))
                .size(36.0).strong().color(color));
            ui.add(egui::ProgressBar::new(f32::from(level) / 100.0).fill(color));
            let label = if reading.confirmed_full() { "Full — Done".to_owned() }
                else if reading.charging == Some(true) && !reading.stale { "⚡ Charging".to_owned() }
                else { reading.state_label().to_owned() };
            ui.label(egui::RichText::new(label).strong().color(color));
            if let Some(mv) = reading.millivolts { ui.label(format!("{mv} mV")); }
            if !reading.stale && reading.charging != Some(true) && !reading.confirmed_full() {
                if let Some(hours) = history.hours_remaining(now) {
                    ui.label(format!("Estimated remaining: {:.1} hours", hours));
                } else {
                    ui.label(egui::RichText::new("Remaining time: collecting discharge data").weak());
                }
            }
        } else {
            ui.label(egui::RichText::new("--%").size(36.0).color(Color32::GRAY));
            ui.label("No current battery reading.");
        }
        if !reading.message.is_empty() { ui.label(egui::RichText::new(&reading.message).weak().small()); }
        ui.add_space(12.0);
        ui.separator();
        ui.label(egui::RichText::new("BATTERY HISTORY  ·  LAST 12 HOURS").small().strong());
        let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 78.0), egui::Sense::hover());
        draw_chart(&history, now, response.rect.shrink2(Vec2::new(4.0, 4.0)), &painter, accent);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("12h ago").weak().small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("Now").weak().small());
            });
        });
        ui.add_space(5.0);
        ui.label(egui::RichText::new(if self.selected == DeviceKind::Headset {
            "Charging state unavailable for G435 • Local data"
        } else { "Device-reported status • Local data" }).weak().small());
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.label(egui::RichText::new("Preferences are saved locally and applied without restarting.").weak());
        ui.add_space(12.0);
        ui.label(egui::RichText::new("GENERAL").small().strong());
        ui.checkbox(&mut self.settings.start_with_windows, "Start with Windows");
        ui.label(egui::RichText::new("Starts at sign-in for your Windows account. Keep the EXE at a fixed location.").small().weak());
        ui.add(egui::Slider::new(&mut self.settings.refresh_seconds, 15..=300).suffix(" s").text("Refresh interval"));
        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("NOTIFICATIONS").small().strong());
        ui.checkbox(&mut self.settings.low_notifications, "Notify when battery is low");
        ui.add_enabled(self.settings.low_notifications, egui::Slider::new(&mut self.settings.low_threshold, 5..=50).suffix("%").text("Low battery"));
        ui.checkbox(&mut self.settings.full_notifications, "Notify when charging is complete");
        ui.label(egui::RichText::new("Full alerts require a confirmed 100% reading after a known charging session (MAD R).").small().weak());
        ui.add_space(10.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Save settings").clicked() {
                let updated = self.settings.clone().validated();
                let before = runtime::startup_enabled().unwrap_or(self.original.start_with_windows);
                match runtime::set_startup(updated.start_with_windows) {
                    Ok(()) => match updated.save() {
                        Ok(()) => {
                            self.original = updated.clone();
                            self.settings = updated;
                            self.message = Some((true, "Settings saved.".into()));
                        }
                        Err(error) => {
                            let rollback = runtime::set_startup(before);
                            self.message = Some((false, format!("Could not save settings: {error}{}", if rollback.is_err() { " (startup change could not be reverted)" } else { "" })));
                        }
                    },
                    Err(error) => self.message = Some((false, format!("Could not change startup: {error}"))),
                }
            }
            if ui.button("Reset defaults").clicked() {
                self.settings = Settings::default();
                self.message = Some((true, "Defaults loaded. Click Save to apply.".into()));
            }
        });
        if self.settings != self.original { ui.label(egui::RichText::new("Unsaved changes").small().weak()); }
        if let Some((success, message)) = &self.message {
            ui.label(egui::RichText::new(message).color(if *success { Color32::LIGHT_GREEN } else { Color32::LIGHT_RED }));
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(concat!("Wireless Battery Monitor v", env!("CARGO_PKG_VERSION"), " • Ahmed Gaber")).small().weak());
    }
}

impl eframe::App for DetailsCard {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());
        ctx.request_repaint_after(Duration::from_secs(2));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                for kind in DeviceKind::ALL {
                    let label = if kind == DeviceKind::Headset { "G435" } else { "MAD R" };
                    if ui.selectable_label(!self.settings_page && self.selected == kind, label).clicked() {
                        self.selected = kind; self.settings_page = false;
                    }
                }
                if ui.selectable_label(self.settings_page, "Settings").clicked() { self.settings_page = true; }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                if self.settings_page { self.settings_page(ui); } else { self.battery_page(ui); }
            });
        });
    }
}

fn draw_chart(history: &BatteryHistory, now: u64, rect: Rect, painter: &egui::Painter, accent: Color32) {
    for fraction in [0.0f32, 0.5, 1.0] {
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
