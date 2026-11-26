//! Independent details and settings window; the monitor remains lightweight.
use crate::runtime;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use wireless_battery_monitor::{
    battery::DeviceKind, diagnostics::Log, history::BatteryHistory,
    preferences::{data_dir, is_portable, Accent, LiveSnapshot, Settings, Theme},
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
        selected: DeviceKind::Headset, page: if open_settings { Page::Settings } else { Page::Battery },
        original: settings.clone(), settings, message: None, notifier: None, icon_texture: None, export_message: None,
    };
    eframe::run_native("Wireless Battery Monitor — Details & Settings", options, Box::new(|_| Ok(Box::new(app))))
        .map_err(|error| std::io::Error::other(error.to_string()).into())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page { Battery, Settings, About }

struct DetailsCard {
    selected: DeviceKind,
    page: Page,
    original: Settings,
    settings: Settings,
    message: Option<(bool, String)>,
    notifier: Option<runtime::Notifier>,
    icon_texture: Option<egui::TextureHandle>,
    export_message: Option<String>,
}

impl DetailsCard {
    fn battery_page(&mut self, ui: &mut egui::Ui) {
        let now = BatteryHistory::now();
        let dark = ui.visuals().dark_mode;
        let accent = if dark { Color32::from_rgb(93, 210, 173) } else { Color32::from_rgb(0, 110, 82) };
        let muted = ui.visuals().weak_text_color();
        let low = if dark { Color32::LIGHT_RED } else { Color32::from_rgb(160, 25, 35) };
        let history = BatteryHistory::load(self.selected);
        let reading = LiveSnapshot::load().reading(self.selected, now);
        ui.heading(self.original.device_name(self.selected));
        ui.label(egui::RichText::new(match self.selected {
            DeviceKind::Headset => "LIGHTSPEED  /  USB receiver",
            DeviceKind::Mouse => "2.4 GHz  /  USB receiver or cable",
        }).weak());
        ui.add_space(12.0);
        if let Some(level) = reading.level {
            let color = if reading.stale { muted }
                else if reading.charging == Some(true) || reading.confirmed_full() { accent }
                else if level <= self.original.profile(self.selected).low_threshold { low } else { accent };
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
        if ui.button("Export history as CSV").clicked() {
            self.export_message = Some(match history.export_csv() {
                Ok(path) => format!("Exported: exports/{}", path.file_name().unwrap_or_default().to_string_lossy()),
                Err(e) => format!("Export failed: {e}"),
            });
        }
        if let Some(message) = &self.export_message { ui.label(egui::RichText::new(message).small()); }
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
        if is_portable() { self.settings.start_with_windows = false; }
        ui.add_enabled(!is_portable(), egui::Checkbox::new(&mut self.settings.start_with_windows, "Start with Windows"));
        ui.label(egui::RichText::new(if is_portable() {
            "Portable mode: local data folder; no startup registration."
        } else { "Starts at sign-in for your Windows account. Keep the EXE at a fixed location." }).small().weak());
        ui.checkbox(&mut self.settings.power_saving, "Power Saving Mode");
        ui.label(egui::RichText::new("Slower background polling (at least 120s), reconnect retries every 45s and no charging animation. Charging remains responsive.").small().weak());
        ui.add(egui::Slider::new(&mut self.settings.refresh_seconds, 15..=300).suffix(" s").text("Refresh interval"));
        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("DEVICES").small().strong());
        ui.checkbox(&mut self.settings.show_headset, "Show Logitech G435");
        ui.checkbox(&mut self.settings.show_mouse, "Show VXE MAD R");
        ui.horizontal(|ui| {
            ui.label("Headset name");
            ui.add(egui::TextEdit::singleline(&mut self.settings.headset_name).hint_text("Logitech G435").char_limit(30));
        });
        ui.horizontal(|ui| {
            ui.label("Mouse name");
            ui.add(egui::TextEdit::singleline(&mut self.settings.mouse_name).hint_text("VXE MAD R").char_limit(30));
        });
        ui.label(egui::RichText::new("Hidden devices still collect history and alerts. If both are hidden, use the notification-area icon.").small().weak());
        for kind in DeviceKind::ALL {
            ui.collapsing(format!("{} profile", kind.name()), |ui| {
                let profile = &mut self.settings.profiles[kind.index()];
                ui.checkbox(&mut profile.custom, "Use independent device settings");
                ui.add_enabled_ui(profile.custom, |ui| {
                    ui.add(egui::Slider::new(&mut profile.refresh_seconds, 15..=300).suffix(" s").text("Refresh"));
                    ui.checkbox(&mut profile.low_notifications, "Low battery alert");
                    ui.add_enabled(profile.low_notifications, egui::Slider::new(&mut profile.low_threshold, 5..=50).suffix("%").text("Low threshold"));
                    if kind == DeviceKind::Mouse { ui.checkbox(&mut profile.full_notifications, "Full battery alert"); }
                });
            });
        }
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
        let secondary = crate::taskbar::secondary_count();
        let selected = self.settings.monitor_slot;
        let label = if selected == 0 { "Primary taskbar".to_owned() }
            else if selected <= secondary { format!("Secondary taskbar {selected}") }
            else { format!("Secondary taskbar {selected} (unavailable)") };
        egui::ComboBox::from_label("Widget monitor").selected_text(label).show_ui(ui, |ui| {
            ui.selectable_value(&mut self.settings.monitor_slot, 0, "Primary taskbar");
            for slot in 1..=secondary {
                ui.selectable_value(&mut self.settings.monitor_slot, slot, format!("Secondary taskbar {slot}"));
            }
        });
        ui.add(egui::Slider::new(&mut self.settings.widget_offset, 0..=240).suffix(" px").text("Left offset"));
        ui.label(egui::RichText::new("Falls back to primary if the selected taskbar is unavailable. Windows must expose a secondary taskbar.").small().weak());
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
        ui.checkbox(&mut self.settings.notification_sound, "Play notification sound (Windows settings apply)");
        if ui.button("Test notification").clicked() {
            self.notifier.get_or_insert_with(runtime::Notifier::new).show("Wireless Battery Monitor", "Test notification — your app is ready.", true, self.settings.notification_sound);
            let _ = Log::Ui.event("notification", "Test requested");
            self.message = Some((true, "Test requested. Check Windows notification settings if it does not appear.".into()));
        }
        ui.label(egui::RichText::new("Full alerts require a confirmed 100% reading after a known charging session (MAD R).").small().weak());
        ui.add_space(10.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Save settings").clicked() {
                let mut updated = self.settings.clone().validated();
                if is_portable() { updated.start_with_windows = false; }
                let before = runtime::startup_enabled().unwrap_or(self.original.start_with_windows);
                match runtime::set_startup(updated.start_with_windows) {
                    Ok(()) => match updated.save() {
                        Ok(()) => {
                            self.original = updated.clone();
                            self.settings = updated;
                            self.message = Some((true, "Settings saved.".into()));
                            let _ = Log::Ui.event("settings", "Settings saved");
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
        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("BACKUP & DIAGNOSTICS").small().strong());
        ui.horizontal_wrapped(|ui| {
            if ui.button("Export saved settings").clicked() {
                self.message = Some(match self.original.backup() {
                    Ok(()) => (true, "Saved settings-backup.json in the data folder.".into()),
                    Err(e) => (false, format!("Backup failed: {e}")),
                });
            }
            if ui.button("Load settings backup").clicked() {
                match Settings::restore_backup() {
                    Ok(mut saved) => {
                        // Loading a backup never changes the actual startup registration.
                        saved.start_with_windows = self.settings.start_with_windows;
                        self.settings = saved;
                        self.message = Some((true, "Backup loaded. Click Save to apply.".into()));
                    },
                    Err(e) => self.message = Some((false, format!("Restore failed: {e}"))),
                }
            }
        });
        if ui.button("Open data and logs folder").clicked() {
            let result = data_dir().and_then(|dir| {
                std::fs::create_dir_all(&dir)?;
                std::process::Command::new("explorer.exe").arg(dir).spawn().map(|_| ())
            });
            if let Err(e) = result { self.message = Some((false, format!("Cannot open folder: {e}"))); }
        }
        if let Some((success, message)) = &self.message {
            ui.label(egui::RichText::new(message).color(if *success { Color32::LIGHT_GREEN } else { Color32::LIGHT_RED }));
        }
    }

    fn about_page(&mut self, ui: &mut egui::Ui) {
        if self.icon_texture.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied([64,64], include_bytes!(concat!(env!("OUT_DIR"), "/app-icon.rgba")));
            self.icon_texture = Some(ui.ctx().load_texture("app-emblem", image, egui::TextureOptions::LINEAR));
        }
        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            if let Some(texture) = &self.icon_texture { ui.image((texture.id(), Vec2::splat(72.0))); }
            ui.heading("Wireless Battery Monitor");
            ui.label(egui::RichText::new(concat!("Version ", env!("CARGO_PKG_VERSION"), "  •  Windows x64")).weak());
            ui.label(if is_portable() { "Portable edition" } else { "Installed / standalone edition" });
            ui.add_space(8.0);
            ui.label("A native Rust battery monitor for Logitech G435 and VXE MAD R.");
            ui.add_space(14.0);
            ui.hyperlink_to("Check for updates on GitHub", "https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor/releases");
            ui.label(egui::RichText::new("Opens your private repository in your browser. No access token is stored.").small().weak());
            ui.hyperlink_to("Project & documentation", "https://github.com/SymbianDevAhmed/Wireless-Battery-Monitor");
            ui.add_space(12.0);
            ui.separator();
            ui.label(egui::RichText::new("Unofficial; not affiliated with Logitech, VXE or ATK.").small().weak());
            ui.add_space(8.0);
            ui.label("Copyright © 2026 Ahmed Gaber");
            ui.hyperlink_to("SymbianDevAhmed", "https://github.com/SymbianDevAhmed");
        });
    }
}

impl eframe::App for DetailsCard {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let light = match self.settings.theme { Theme::System => runtime::system_light_theme(), Theme::Light => true, Theme::Dark => false };
        ctx.set_visuals(if light { egui::Visuals::light() } else { egui::Visuals::dark() });
        ctx.request_repaint_after(Duration::from_secs(2));
        if let Some(notifier) = self.notifier.as_mut() { notifier.tick(); }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                for kind in DeviceKind::ALL {
                    let label = if kind == DeviceKind::Headset { "G435" } else { "MAD R" };
                    if ui.selectable_label(self.page == Page::Battery && self.selected == kind, label).clicked() {
                        self.selected = kind; self.page = Page::Battery; self.export_message = None;
                    }
                }
                if ui.selectable_label(self.page == Page::Settings, "Settings").clicked() { self.page = Page::Settings; }
                if ui.selectable_label(self.page == Page::About, "About").clicked() { self.page = Page::About; }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                match self.page { Page::Settings => self.settings_page(ui), Page::About => self.about_page(ui), Page::Battery => self.battery_page(ui) }
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
