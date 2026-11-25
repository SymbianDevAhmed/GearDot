//! Independent details and settings window; the monitor remains lightweight.
use crate::runtime;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use wireless_battery_monitor::{
    battery::DeviceKind, history::BatteryHistory,
    preferences::{Accent, LiveSnapshot, Settings, Theme},
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
        let dark = ui.visuals().dark_mode;
        let accent = if dark { Color32::from_rgb(93, 210, 173) } else { Color32::from_rgb(0, 110, 82) };
        let muted = ui.visuals().weak_text_color();
        let low = if dark { Color32::LIGHT_RED } else { Color32::from_rgb(160, 25, 35) };
        let history = BatteryHistory::load(self.selected);
        let reading = LiveSnapshot::load().reading(self.selected, now);
        ui.heading(self.selected.name());
        ui.label(egui::RichText::new(match self.selected {
            DeviceKind::Headset => "LIGHTSPEED  /  USB receiver",
            DeviceKind::Mouse => "2.4 GHz  /  USB receiver or cable",
        }).weak());
        ui.add_space(12.0);
        if let Some(level) = reading.level {
            let color = if reading.stale { muted }
                else if reading.charging == Some(true) || reading.confirmed_full() { accent }
                else if level <= Settings::load().low_threshold { low } else { accent };
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
        ui.label(egui::RichText::new(format!("BATTERY HISTORY  ·  LAST {} HOURS", self.original.chart_hours)).small().strong());
        let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 118.0), egui::Sense::hover());
        let mut plot = response.rect.shrink2(Vec2::new(4.0, 8.0));
        plot.min.x += 29.0;
        draw_chart(&history, now, self.original.chart_hours, plot, &painter, accent, muted);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{}h ago", self.original.chart_hours)).weak().small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("Now").weak().small());
            });
        });
        if let Some(stats) = history.statistics(now, self.original.chart_hours) {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Min {}%", stats.minimum));
                ui.label(format!("Max {}%", stats.maximum));
                ui.label(format!("Net {:+} pp", stats.net_change));
            });
            ui.label(egui::RichText::new(format!("{} recorded samples", stats.count)).weak().small());
        }
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
        ui.label(egui::RichText::new("DEVICES").small().strong());
        ui.checkbox(&mut self.settings.show_headset, "Show Logitech G435");
        ui.checkbox(&mut self.settings.show_mouse, "Show VXE MAD R");
        ui.label(egui::RichText::new("Hidden devices still collect history and alerts. If both are hidden, use the notification-area icon.").small().weak());
        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("APPEARANCE").small().strong());
        egui::ComboBox::from_label("Window theme").selected_text(self.settings.theme.label()).show_ui(ui, |ui| {
            for theme in Theme::ALL { ui.selectable_value(&mut self.settings.theme, theme, theme.label()); }
        });
        egui::ComboBox::from_label("Widget colour").selected_text(self.settings.widget_accent.label()).show_ui(ui, |ui| {
            for accent in Accent::ALL { ui.selectable_value(&mut self.settings.widget_accent, accent, accent.label()); }
        });
        ui.add(egui::Slider::new(&mut self.settings.widget_scale, 80..=130).suffix("%").text("Widget size"));
        ui.checkbox(&mut self.settings.animate_charging, "Animate charging indicator");
        ui.horizontal(|ui| {
            ui.label("History range");
            egui::ComboBox::from_id_salt("history_range").selected_text(format!("{} hours", self.settings.chart_hours)).show_ui(ui, |ui| {
                for hours in [3,6,12,24,48] { ui.selectable_value(&mut self.settings.chart_hours, hours, format!("{hours}h")); }
            });
        });
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
        let light = match self.settings.theme { Theme::System => runtime::system_light_theme(), Theme::Light => true, Theme::Dark => false };
        ctx.set_visuals(if light { egui::Visuals::light() } else { egui::Visuals::dark() });
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

fn draw_chart(history: &BatteryHistory, now: u64, hours: u8, rect: Rect, painter: &egui::Painter, accent: Color32, muted: Color32) {
    for fraction in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let y = rect.bottom() - rect.height() * fraction;
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(0.5, muted.gamma_multiply(0.45)));
        painter.text(Pos2::new(rect.left()-3.0, y), egui::Align2::RIGHT_CENTER,
            format!("{:.0}%", fraction * 100.0), egui::FontId::proportional(10.0), muted);
    }
    let span = u64::from(hours) * 3600;
    let start = now.saturating_sub(span);
    let mut previous: Option<(u64, Pos2)> = None;
    for sample in history.samples().iter().filter(|s| s.at >= start && s.at <= now) {
        let x = rect.left() + (sample.at - start) as f32 / span as f32 * rect.width();
        let y = rect.bottom() - f32::from(sample.percent) / 100.0 * rect.height();
        let point = Pos2::new(x, y);
        let color = if sample.charging == Some(true) { Color32::from_rgb(70, 155, 85) } else { accent };
        if let Some((at, pos)) = previous {
            if sample.at - at <= 600 {
                painter.line_segment([pos, point], Stroke::new(2.0, color));
            }
        }
        painter.circle_filled(point, 2.0, color);
        previous = Some((sample.at, point));
    }
    if previous.is_none() {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Waiting for data",
            egui::FontId::proportional(12.0), muted);
    }
}
