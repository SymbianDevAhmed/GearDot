//! Independent details and settings window; the monitor remains lightweight.
use crate::{
    background,
    icon::DeviceGlyph,
    runtime,
    taskbar::{self, WidgetPreview, WidgetReading},
    window_motion::WindowMotion,
};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use std::time::{Duration, Instant};
use wireless_battery_monitor::{
    battery::{Connection, DeviceKind},
    diagnostics::Log,
    palette::{Palette, Rgb, VisualTheme},
    preferences::{
        data_dir, device_order, is_portable, live_data_dir, unix_now, Accent, EdgeSide,
        InputCharge, InputStatus, LiveSnapshot, Settings, TemporaryMute, Theme,
    },
};

const NAVIGATION_WIDTH: f32 = 88.0;
const NAVIGATION_PADDING: i8 = 8;
const WINDOW_SIZE: [f32; 2] = [470.0, 664.0];
const WINDOW_RADIUS: f32 = 16.0;
const WORKSPACE_RADIUS: f32 = 14.0;
const CORNER_STEPS: usize = 12;

pub fn show(open_settings: bool, demo: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size([422.0, 480.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_icon(egui::IconData {
            rgba: include_bytes!(concat!(env!("OUT_DIR"), "/app-icon.rgba")).to_vec(),
            width: 64,
            height: 64,
        })
        .with_always_on_top();
    let mut settings = Settings::load();
    if let Ok(enabled) = runtime::startup_enabled() {
        settings.start_with_windows = enabled;
    }
    if !open_settings && !demo {
        if let Some([x, y]) = crate::taskbar::overview_position(
            settings.monitor_slot,
            WINDOW_SIZE[0] as i32,
            WINDOW_SIZE[1] as i32,
        ) {
            viewport = viewport.with_position(Pos2::new(x, y));
        }
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let app = details_card(settings, open_settings, demo);
    eframe::run_native(
        runtime::DETAILS_TITLE,
        options,
        Box::new(|creation| {
            install_heading_font(&creation.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| std::io::Error::other(error.to_string()).into())
}

fn details_card(settings: Settings, open_settings: bool, demo: bool) -> DetailsCard {
    DetailsCard {
        page: if open_settings {
            Page::Settings
        } else {
            Page::Overview
        },
        original: settings.clone(),
        settings,
        message: None,
        notifier: None,
        icon_texture: None,
        popup: !open_settings && !demo,
        demo,
        backdrop: None,
        glass_active: false,
        animations: runtime::animations_enabled(),
        motion: None,
        background_id: None,
        background_texture: None,
        window_icons: runtime::WindowIcons::default(),
        widget_previews: Default::default(),
    }
}

fn age_label(seconds: u64) -> String {
    if seconds < 60 {
        format!("Last read {seconds}s ago")
    } else if seconds < 3600 {
        format!("Last read {} min ago", seconds / 60)
    } else {
        format!("Last read {}h ago", seconds / 3600)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    QuickControls,
    Diagnostics,
    Settings,
    About,
}

struct DetailsCard {
    page: Page,
    original: Settings,
    settings: Settings,
    message: Option<(bool, String)>,
    notifier: Option<runtime::Notifier>,
    icon_texture: Option<egui::TextureHandle>,
    popup: bool,
    demo: bool,
    backdrop: Option<(bool, bool, u8)>,
    glass_active: bool,
    animations: bool,
    motion: Option<WindowMotion>,
    background_id: Option<String>,
    background_texture: Option<egui::TextureHandle>,
    window_icons: runtime::WindowIcons,
    widget_previews: [WidgetPreview; 3],
}

#[derive(Clone, PartialEq, Eq)]
enum ProfileKey {
    BuiltIn(DeviceKind),
    Detected(String),
}

impl ProfileKey {
    fn id(&self) -> &str {
        match self {
            Self::BuiltIn(kind) => kind.profile_id(),
            Self::Detected(id) => id,
        }
    }
}

struct DeviceCard {
    profile: ProfileKey,
    name: String,
    kind: DeviceGlyph,
    level: Option<u8>,
    coarse: Option<&'static str>,
    connection: Connection,
    status: String,
    battery_note: Option<&'static str>,
    colour: Color32,
    approximate: bool,
    stale_age: Option<u64>,
}

fn colour(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

fn request_device_refresh() -> std::io::Result<()> {
    let dir = live_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("refresh.request"), b"")
}

fn current_snapshot() -> LiveSnapshot {
    if runtime::monitor_running() {
        LiveSnapshot::load()
    } else {
        LiveSnapshot::default()
    }
}

fn heading_family() -> egui::FontFamily {
    egui::FontFamily::Name("GearDot heading".into())
}

fn install_heading_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut family = fonts.families[&egui::FontFamily::Proportional].clone();
    if let Some(windows) = std::env::var_os("SystemRoot") {
        let directory = std::path::PathBuf::from(windows).join("Fonts");
        if let Some(data) = ["seguisb.ttf", "segoeuib.ttf"]
            .iter()
            .find_map(|file| std::fs::read(directory.join(file)).ok())
        {
            fonts.font_data.insert(
                "GearDot semibold".into(),
                egui::FontData::from_owned(data).into(),
            );
            family.insert(0, "GearDot semibold".into());
        }
    }
    fonts.families.insert(heading_family(), family);
    ctx.set_fonts(fonts);
}

fn heading(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .size(23.0)
        .family(heading_family())
}

fn themed_visuals(palette: &Palette, light: bool) -> egui::Visuals {
    let mut visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.panel_fill = colour(palette.background);
    visuals.window_fill = colour(palette.background);
    visuals.faint_bg_color = colour(palette.surface);
    visuals.extreme_bg_color = colour(palette.surface);
    visuals.hyperlink_color = colour(palette.accent);
    visuals.warn_fg_color = colour(palette.low);
    visuals.error_fg_color = colour(palette.low);
    visuals.selection.bg_fill = colour(palette.selection);
    visuals.selection.stroke = Stroke::new(1.0_f32, colour(palette.text));
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = colour(palette.surface);
        widget.weak_bg_fill = colour(palette.surface);
        widget.bg_stroke.color = colour(palette.border);
        widget.fg_stroke.color = colour(palette.text);
    }
    for widget in [
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = colour(palette.selection);
        widget.weak_bg_fill = colour(palette.selection);
        widget.bg_stroke.color = colour(palette.accent);
    }
    visuals
}

fn device_glyph(
    painter: &egui::Painter,
    center: Pos2,
    kind: DeviceGlyph,
    colour: Color32,
    cutout: Color32,
) {
    let stroke = Stroke::new(2.2_f32, colour);
    let point = |x, y| center + Vec2::new(x, y);
    match kind {
        DeviceGlyph::Headset => {
            let points = (0..=24)
                .map(|i| {
                    let angle = std::f32::consts::PI * (1.0 + i as f32 / 24.0);
                    point(angle.cos() * 8.0, angle.sin() * 8.0)
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
            for x in [-7.5, 7.5] {
                painter.rect_filled(
                    Rect::from_center_size(point(x, 3.0), Vec2::new(4.0, 8.0)),
                    1.5,
                    colour,
                );
            }
        }
        DeviceGlyph::Earbuds => {
            for x in [-5.25, 5.25] {
                painter.circle_filled(point(x, -4.5), 3.6, colour);
                painter.line_segment([point(x, -1.0), point(x, 6.5)], stroke);
                painter.circle_filled(point(x, 6.5), 1.0, colour);
            }
        }
        DeviceGlyph::Mouse => {
            painter.rect_filled(
                Rect::from_center_size(center, Vec2::new(13.0, 19.0)),
                6.0,
                colour,
            );
            painter.line_segment(
                [point(0.0, -6.0), point(0.0, -1.0)],
                Stroke::new(2.0_f32, cutout),
            );
        }
        DeviceGlyph::Keyboard => {
            painter.rect_stroke(
                Rect::from_min_max(point(-11.0, -6.0), point(11.0, 6.0)),
                3,
                stroke,
                egui::StrokeKind::Inside,
            );
            for x in [-6.0, -2.0, 2.0, 6.0] {
                painter.circle_filled(point(x, -1.5), 0.9, colour);
            }
            painter.line_segment(
                [point(-4.0, 2.5), point(4.0, 2.5)],
                Stroke::new(1.2_f32, colour),
            );
        }
        DeviceGlyph::Gamepad => {
            let outline = [
                (-11.0, 5.0),
                (-10.0, -2.0),
                (-8.0, -6.0),
                (-5.0, -8.0),
                (5.0, -8.0),
                (8.0, -6.0),
                (10.0, -2.0),
                (11.0, 5.0),
                (9.0, 8.0),
                (6.0, 7.0),
                (2.0, 3.0),
                (-2.0, 3.0),
                (-6.0, 7.0),
                (-9.0, 8.0),
            ];
            painter.add(egui::Shape::closed_line(
                outline.into_iter().map(|(x, y)| point(x, y)).collect(),
                stroke,
            ));
            painter.line_segment(
                [point(-7.0, 0.0), point(-2.0, 0.0)],
                Stroke::new(1.5_f32, colour),
            );
            painter.line_segment(
                [point(-4.5, -2.5), point(-4.5, 2.5)],
                Stroke::new(1.5_f32, colour),
            );
            painter.circle_filled(point(4.0, -1.5), 1.25, colour);
            painter.circle_filled(point(7.0, 1.0), 1.25, colour);
        }
        _ => {
            painter.line_segment([point(0.0, -9.0), point(0.0, 9.0)], stroke);
            painter.add(egui::Shape::line(
                vec![point(-6.0, -6.0), point(6.0, 4.0), point(0.0, 9.0)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![point(-6.0, 6.0), point(6.0, -4.0), point(0.0, -9.0)],
                stroke,
            ));
        }
    }
}

fn draw_device_card(ui: &mut egui::Ui, card: &DeviceCard, palette: &Palette, pinned: bool) {
    let fill = ui.visuals().widgets.noninteractive.bg_fill;
    egui::Frame::new()
        .fill(fill)
        .corner_radius(13)
        .inner_margin(11)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (response, painter) =
                    ui.allocate_painter(Vec2::splat(29.0), egui::Sense::hover());
                device_glyph(
                    &painter,
                    response.rect.center(),
                    card.kind,
                    colour(card.kind.tint(!ui.visuals().dark_mode)),
                    fill,
                );
                ui.vertical(|ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let name = egui::Label::new(
                            egui::RichText::new(&card.name)
                                .size(16.0)
                                .family(heading_family()),
                        )
                        .truncate();
                        let level = card
                            .level
                            .map(|n| format!("{}{n}%", if card.approximate { "~" } else { "" }))
                            .or_else(|| card.coarse.map(str::to_owned));
                        let level_width = if level.is_some() { 65.0 } else { 4.0 };
                        ui.allocate_ui_with_layout(
                            Vec2::new((ui.available_width() - level_width).max(0.0), 22.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add(name);
                            },
                        );
                        if let Some(level) = level {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(level).color(card.colour).strong(),
                                    )
                                    .on_hover_text(&card.status);
                                },
                            );
                        }
                    });
                    if let Some(level) = card.level {
                        let (bar, painter) = ui.allocate_painter(
                            Vec2::new(ui.available_width(), 4.0),
                            egui::Sense::hover(),
                        );
                        painter.rect_filled(bar.rect, 2, colour(palette.border));
                        if level > 0 {
                            let width = bar.rect.width() * f32::from(level.min(100)) / 100.0;
                            painter.rect_filled(
                                Rect::from_min_size(
                                    bar.rect.min,
                                    Vec2::new(width, bar.rect.height()),
                                ),
                                2,
                                card.colour,
                            );
                        }
                    }
                    let subtitle = format!("{} · {}", card.kind.label(), card.connection.label());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(subtitle)
                                .small()
                                .color(colour(palette.muted)),
                        )
                        .on_hover_text(&card.status);
                        if pinned {
                            ui.label(egui::RichText::new("Pinned").small().weak());
                        }
                        if let Some(age) = card.stale_age {
                            ui.label(
                                egui::RichText::new(age_label(age))
                                    .small()
                                    .color(card.colour),
                            );
                        } else if let Some(note) = card.battery_note {
                            ui.label(egui::RichText::new(note).small().color(card.colour));
                        }
                    });
                });
            });
        });
}

impl DetailsCard {
    fn request_close(&mut self, ctx: &egui::Context, now: Instant) {
        if self.animations {
            self.motion.get_or_insert(WindowMotion::Still).close(now);
            ctx.request_repaint();
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn background_tint(&self) -> Color32 {
        Color32::WHITE.gamma_multiply(f32::from(self.settings.background_opacity) / 100.0)
    }

    fn palette(&self, ui: &egui::Ui) -> &'static Palette {
        self.settings.visual_theme.palette(!ui.visuals().dark_mode)
    }
    fn device_cards(
        &self,
        palette: &Palette,
        snapshot: &LiveSnapshot,
        now: u64,
    ) -> Vec<DeviceCard> {
        let mut cards = Vec::new();
        for (index, kind) in DeviceKind::ALL.into_iter().enumerate() {
            let mut reading = snapshot.reading(kind, now);
            if self.demo {
                reading = wireless_battery_monitor::preferences::LiveReading {
                    level: Some([18, 64][index]),
                    checked_at: now,
                    connection: [Connection::Dongle, Connection::Wired][index],
                    charging: (index == 1).then_some(true),
                    ..Default::default()
                };
            }
            let Some(level) = reading.level.filter(|_| reading.online()) else {
                continue;
            };
            let low_threshold = self.original.profile(kind).low_threshold;
            let is_low = reading.charging != Some(true) && level <= low_threshold;
            let status = if is_low {
                "Low battery"
            } else {
                reading.state_label()
            }
            .into();
            let battery_note = if reading.confirmed_full() {
                Some("Full")
            } else if reading.charging == Some(true) {
                Some("Charging")
            } else if is_low {
                Some("Low battery")
            } else {
                None
            };
            cards.push(DeviceCard {
                profile: ProfileKey::BuiltIn(kind),
                name: kind.name().into(),
                kind: if kind == DeviceKind::Headset {
                    DeviceGlyph::Headset
                } else {
                    DeviceGlyph::Mouse
                },
                level: Some(level),
                coarse: None,
                connection: reading.connection,
                status,
                battery_note,
                colour: colour(palette.battery_colour(
                    level,
                    low_threshold,
                    reading.charging == Some(true),
                    reading.confirmed_full(),
                )),
                approximate: reading.estimated,
                stale_age: None,
            });
        }
        for reading in snapshot.connected_bluetooth(now) {
            let kind = DeviceGlyph::for_bluetooth(&reading.name, reading.kind);
            let threshold = self.original.low_threshold;
            let battery_note = reading
                .percent
                .is_some_and(|percent| percent <= threshold)
                .then_some("Low battery");
            let battery_colour = reading.percent.map_or(palette.muted, |level| {
                palette.battery_level(level, threshold)
            });
            let status = if reading.percent.is_some() {
                "Bluetooth · Connected"
            } else {
                "Bluetooth · Battery unavailable"
            };
            cards.push(DeviceCard {
                profile: ProfileKey::Detected(reading.profile_id().into_owned()),
                name: reading.name.clone(),
                kind,
                level: reading.percent,
                coarse: None,
                connection: Connection::Bluetooth,
                status: status.into(),
                battery_note,
                colour: colour(battery_colour),
                approximate: false,
                stale_age: None,
            });
        }
        if self.demo {
            cards.push(DeviceCard {
                profile: ProfileKey::Detected("demo-earbuds".into()),
                name: "Bluetooth earbuds (sample)".into(),
                kind: DeviceGlyph::Earbuds,
                level: Some(46),
                coarse: None,
                connection: Connection::Bluetooth,
                status: "Bluetooth · Sample reading".into(),
                battery_note: None,
                colour: colour(palette.battery_level(46, self.original.low_threshold)),
                approximate: false,
                stale_age: None,
            });
        }
        for input in snapshot.connected_inputs(now) {
            let kind = DeviceGlyph::for_detected(&input.name, input.kind, input.connection());
            let connection = input.connection();
            let coarse = input.status.level_label();
            let mut status = if coarse.is_some() {
                "Battery level".to_owned()
            } else {
                match input.status {
                    InputStatus::Wired | InputStatus::WiredOnly => "Wired".into(),
                    InputStatus::Receiver => "Peripheral status unavailable".into(),
                    _ => "Battery unavailable".into(),
                }
            };
            let mut ink = match coarse {
                Some("Empty" | "Low") => palette.low,
                Some("Medium") => palette.battery_level(60, self.original.low_threshold),
                Some("Full") => palette.full,
                _ => palette.muted,
            };
            let mut note = None;
            let mut stale_age = None;
            if let Some(battery) = input.battery {
                if battery.stale {
                    note = Some("Last reading");
                    stale_age = Some(now.saturating_sub(battery.checked_at));
                } else {
                    ink = palette.battery_colour(
                        battery.percent,
                        self.original.low_threshold,
                        battery.charge == InputCharge::Charging,
                        battery.charge == InputCharge::Full,
                    );
                    note = match battery.charge {
                        InputCharge::Full => Some("Full"),
                        InputCharge::Charging => Some("Charging"),
                        InputCharge::Discharging
                            if battery.percent <= self.original.low_threshold =>
                        {
                            Some("Low battery")
                        }
                        InputCharge::Discharging => None,
                    };
                }
                status = if battery.estimated {
                    "Estimated from device battery level".into()
                } else {
                    "Device-reported".into()
                };
            }
            cards.push(DeviceCard {
                profile: ProfileKey::Detected(input.profile_id().into_owned()),
                name: input.name.clone(),
                kind,
                level: input.battery.map(|reading| reading.percent),
                coarse,
                connection,
                status,
                battery_note: note,
                colour: colour(ink),
                approximate: input.battery.is_some_and(|reading| reading.estimated),
                stale_age,
            });
        }
        cards.sort_by_key(|card| (card.connection, card.level.unwrap_or(u8::MAX)));
        cards
    }

    fn overview_heading(&self, ui: &mut egui::Ui, title: egui::RichText, palette: &Palette) {
        let title = title.color(colour(palette.text));
        if (self.background_texture.is_some() && self.settings.background_opacity > 0)
            || self.settings.current_theme_transparency() > 0
        {
            egui::Frame::new()
                .fill(colour(palette.surface))
                .corner_radius(7)
                .inner_margin(egui::Margin::symmetric(7, 3))
                .show(ui, |ui| ui.label(title));
        } else {
            ui.label(title);
        }
    }

    fn visible_cards(
        &self,
        palette: &Palette,
        snapshot: &LiveSnapshot,
        now: u64,
    ) -> Vec<DeviceCard> {
        let mut cards = self.device_cards(palette, snapshot, now);
        cards.retain_mut(|card| {
            if !self.demo && !self.original.placement(card.profile.id()).devices {
                return false;
            }
            match &card.profile {
                ProfileKey::BuiltIn(kind) => {
                    if !self.demo && !self.original.widget_options().visible[kind.index()] {
                        return false;
                    }
                    card.name = self.original.device_name(*kind).to_owned();
                }
                ProfileKey::Detected(id) => {
                    if !self.demo && !self.original.detected_visible(id) {
                        return false;
                    }
                    card.name = self.original.detected_name(id, &card.name).to_owned();
                }
            }
            true
        });
        cards.sort_by(|left, right| {
            let key = |card: &DeviceCard| {
                device_order(
                    self.original.placement(card.profile.id()).pinned,
                    card.battery_note == Some("Low battery")
                        || matches!(card.coarse, Some("Low" | "Empty")),
                    card.level,
                )
            };
            key(left)
                .cmp(&key(right))
                .then_with(|| left.name.cmp(&right.name))
        });
        cards
    }

    fn overview(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette(ui);
        let refresh = ui
            .horizontal(|ui| {
                self.overview_heading(
                    ui,
                    heading(if self.demo {
                        "Device preview"
                    } else {
                        "Devices"
                    })
                    .size(26.0),
                    palette,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    !self.demo
                        && ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("↻")
                                        .size(17.0)
                                        .color(colour(palette.accent)),
                                )
                                .min_size(Vec2::splat(25.0))
                                .corner_radius(5)
                                .fill(colour(palette.selection))
                                .stroke(Stroke::new(1.0_f32, colour(palette.border))),
                            )
                            .on_hover_text("Refresh batteries")
                            .clicked()
                })
                .inner
            })
            .inner;
        if refresh {
            let _ = request_device_refresh();
        }
        let snapshot = if self.demo {
            LiveSnapshot::default()
        } else {
            current_snapshot()
        };
        let cards = self.visible_cards(palette, &snapshot, unix_now());
        if cards.is_empty() {
            self.overview_heading(
                ui,
                egui::RichText::new("Waiting for a connected device."),
                palette,
            );
        }
        ui.add_space(9.0);
        for card in &cards {
            let pinned = self.original.placement(card.profile.id()).pinned;
            draw_device_card(ui, card, palette, pinned);
            ui.add_space(6.0);
        }
    }

    fn quick_controls_page(&mut self, ui: &mut egui::Ui) {
        ui.label(heading("Quick Controls"));
        ui.spacing_mut().item_spacing.y = 10.0;
        let mut updated = self.original.clone();
        ui.checkbox(
            &mut updated.taskbar_widget,
            "Show battery indicator on the taskbar",
        );
        ui.checkbox(&mut updated.edge_widget, "Show screen-edge battery slider");
        ui.checkbox(
            &mut updated.battery_alerts_muted,
            "Mute battery and connection alerts",
        );
        self.save_quick_controls(updated);
        ui.separator();
        ui.label(heading("Temporary mute").size(17.0));
        let now = unix_now();
        let snapshot = current_snapshot();
        let mute = TemporaryMute::load();
        let active = mute.active(now, &snapshot.monitor_session);
        if active {
            let label = match &mute {
                TemporaryMute::Until(end) => format!(
                    "Muted for {} more min",
                    end.saturating_sub(now).div_ceil(60)
                ),
                _ => "Muted until GearDot exits".into(),
            };
            ui.label(label);
        }
        ui.horizontal_wrapped(|ui| {
            let request = if ui.button("Mute for 1 hour").clicked() {
                Some(TemporaryMute::Until(now + 3600))
            } else if ui
                .add_enabled(
                    !snapshot.monitor_session.is_empty(),
                    egui::Button::new("Mute until exit"),
                )
                .clicked()
            {
                Some(TemporaryMute::Session(snapshot.monitor_session))
            } else if ui
                .add_enabled(active, egui::Button::new("End temporary mute"))
                .clicked()
            {
                Some(TemporaryMute::Off)
            } else {
                None
            };
            if let Some(mute) = request {
                self.set_temporary_mute(mute);
            }
        });
        self.show_message(ui);
    }

    fn save_quick_controls(&mut self, updated: Settings) {
        if updated != self.original {
            match updated.save() {
                Ok(()) => {
                    // Keep unrelated edits in the Settings draft.
                    if updated.taskbar_widget != self.original.taskbar_widget {
                        self.settings.taskbar_widget = updated.taskbar_widget;
                    }
                    if updated.edge_widget != self.original.edge_widget {
                        self.settings.edge_widget = updated.edge_widget;
                    }
                    if updated.battery_alerts_muted != self.original.battery_alerts_muted {
                        self.settings.battery_alerts_muted = updated.battery_alerts_muted;
                        if updated.battery_alerts_muted {
                            if let Some(notifier) = self.notifier.as_mut() {
                                notifier.clear();
                            }
                        }
                    }
                    self.original = updated;
                    self.message = Some((true, "Saved.".into()));
                }
                Err(error) => self.message = Some((false, format!("Could not save: {error}"))),
            }
        }
    }

    fn set_temporary_mute(&mut self, mute: TemporaryMute) {
        self.message = Some(match mute.save() {
            Ok(()) => {
                if mute != TemporaryMute::Off {
                    if let Some(notifier) = self.notifier.as_mut() {
                        notifier.clear();
                    }
                }
                (true, "Temporary mute updated.".into())
            }
            Err(error) => (false, format!("Could not update temporary mute: {error}")),
        });
    }

    fn show_message(&self, ui: &mut egui::Ui) {
        if let Some((ok, message)) = &self.message {
            let palette = self.palette(ui);
            ui.colored_label(
                colour(if *ok { palette.full } else { palette.low }),
                message,
            );
        }
    }

    fn diagnostics_page(&mut self, ui: &mut egui::Ui) {
        let snapshot = current_snapshot();
        let now = unix_now();
        ui.horizontal_wrapped(|ui| {
            ui.label(heading("Diagnostics"));
            if ui.button("Rescan devices").clicked() {
                self.message = Some(match request_device_refresh() {
                    Ok(()) => (true, "Refresh requested.".into()),
                    Err(error) => (false, format!("Could not request refresh: {error}")),
                });
            }
            if ui.button("Copy diagnostic report").clicked() {
                ui.ctx()
                    .copy_text(wireless_battery_monitor::diagnostics::report(
                        &snapshot, now,
                    ));
                self.message = Some((true, "Report copied.".into()));
            }
        });
        self.show_message(ui);
        let cards = self.device_cards(self.palette(ui), &snapshot, now);
        ui.add_space(8.0);
        ui.label(heading("Windows device detection").size(17.0));
        let scan_status = |timestamp, expiry| {
            let elapsed = now.saturating_sub(timestamp);
            match (timestamp, elapsed) {
                (0, _) => "Waiting for first scan".into(),
                (_, seconds) if seconds > expiry => format!("Last scan {seconds}s ago · stale"),
                (_, seconds) => format!("Scanned {seconds}s ago"),
            }
        };
        let bluetooth = snapshot.connected_bluetooth(now).count();
        let inputs = snapshot.connected_inputs(now).len();
        ui.label(format!(
            "Bluetooth · {bluetooth} detected · {}",
            scan_status(snapshot.bluetooth_checked_at, 45)
        ));
        if let Some(error) = &snapshot.bluetooth_error {
            ui.colored_label(
                colour(self.palette(ui).low),
                format!("Bluetooth scan failed: {error}"),
            );
        }
        ui.label(format!(
            "USB / Audio / XInput · {inputs} detected · {}",
            scan_status(snapshot.inputs_checked_at, 60)
        ));
        if let Some(error) = &snapshot.inputs_hid_error {
            ui.colored_label(
                colour(self.palette(ui).low),
                format!("USB HID scan failed: {error}"),
            );
        }
        if let Some(error) = &snapshot.audio_error {
            ui.colored_label(
                colour(self.palette(ui).low),
                format!("Windows Audio scan failed: {error}"),
            );
        }
        for card in cards
            .iter()
            .filter(|card| matches!(card.profile, ProfileKey::Detected(_)))
        {
            let source = match &card.profile {
                _ if card.connection == Connection::Bluetooth => "Windows Bluetooth",
                ProfileKey::Detected(id) if id.starts_with("xinput-") => "Windows XInput",
                ProfileKey::Detected(id) if id.starts_with("audio-") => "Windows Audio",
                _ => "Windows USB HID",
            };
            let battery = match (card.level, card.coarse) {
                (Some(level), _) => format!("{level}% · {source}"),
                (_, Some(level)) => format!("{level} · no percentage provided"),
                _ => "Battery level unavailable".into(),
            };
            ui.separator();
            ui.label(egui::RichText::new(&card.name).strong());
            ui.label(egui::RichText::new(format!("{source} · {}", card.connection.label())).weak());
            ui.label(battery);
        }
        let readers = DeviceKind::ALL.map(|kind| (kind, snapshot.reading(kind, now)));
        if readers.iter().any(|(_, reading)| reading.online()) {
            ui.add_space(12.0);
            ui.label(heading("Device battery readers").size(17.0));
        }
        for (kind, reading) in readers {
            let Some(level) = reading.level.filter(|_| reading.online()) else {
                continue;
            };
            ui.separator();
            ui.label(egui::RichText::new(kind.name()).strong());
            let battery = if reading.estimated {
                format!("~{level}% estimated from voltage")
            } else {
                format!("{level}% reported by the device")
            };
            ui.label(format!("{} · {battery}", reading.connection.label()));
        }
    }

    fn widget_preview(&mut self, ui: &mut egui::Ui, readings: &[WidgetReading]) {
        let options = self.settings.widget_options();
        let taskbar_light = taskbar::light_taskbar();
        let sample = |edge| {
            let mut selected = taskbar::readings_for_display(readings, &self.settings, edge);
            let example = readings.is_empty();
            if example {
                selected = vec![
                    WidgetReading {
                        glyph: DeviceGlyph::Headset,
                        level: 18,
                        low_threshold: self.settings.low_threshold,
                        ..Default::default()
                    },
                    WidgetReading {
                        glyph: DeviceGlyph::Mouse,
                        level: 76,
                        low_threshold: self.settings.low_threshold,
                        ..Default::default()
                    },
                ];
            }
            selected.truncate(2);
            (selected, example)
        };
        let (bar_readings, example) = sample(false);
        let (edge_readings, _) = sample(true);
        ui.separator();
        ui.label(
            heading(if example {
                "Widget preview · Sample devices"
            } else {
                "Widget preview"
            })
            .size(17.0),
        );

        ui.label("Taskbar");
        if options.taskbar && !bar_readings.is_empty() {
            let texture = self.widget_previews[0].render(
                ui.ctx(),
                options,
                bar_readings,
                false,
                false,
                taskbar_light,
            );
            let (rect, painter) =
                ui.allocate_painter(Vec2::new(ui.available_width(), 34.0), egui::Sense::hover());
            let shell = if taskbar_light {
                Color32::from_rgb(239, 239, 243)
            } else {
                Color32::from_rgb(27, 27, 31)
            };
            painter.rect_filled(rect.rect, 6, shell);
            let anchor = rect.rect.right() - 17.0;
            painter.line_segment(
                [
                    Pos2::new(anchor, rect.rect.top() + 8.0),
                    Pos2::new(anchor, rect.rect.bottom() - 8.0),
                ],
                Stroke::new(1.0_f32, colour(Palette::for_theme(taskbar_light).muted)),
            );
            if let Some(texture) = texture {
                let size = texture.size_vec2();
                let left =
                    (anchor - size.x - f32::from(options.offset)).max(rect.rect.left() + 4.0);
                let image =
                    Rect::from_min_size(Pos2::new(left, rect.rect.center().y - size.y / 2.0), size);
                painter.image(
                    texture.id(),
                    image,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else {
            ui.weak(if options.taskbar {
                "No devices assigned to the taskbar."
            } else {
                "Taskbar indicator is off."
            });
        }

        ui.label("Edge slider");
        if options.edge && !edge_readings.is_empty() {
            let palette = self.palette(ui);
            let [_, grip_preview, panel_preview] = &mut self.widget_previews;
            let grip =
                grip_preview.render(ui.ctx(), options, Vec::new(), true, false, taskbar_light);
            let panel =
                panel_preview.render(ui.ctx(), options, edge_readings, true, true, taskbar_light);
            let height = panel
                .map_or(96.0, |texture| texture.size_vec2().y + 22.0)
                .max(96.0);
            ui.columns(2, |columns| {
                for (column, (label, texture)) in columns
                    .iter_mut()
                    .zip([("Collapsed", grip), ("Expanded", panel)])
                {
                    column.weak(label);
                    let (response, painter) = column.allocate_painter(
                        Vec2::new(column.available_width(), height),
                        egui::Sense::hover(),
                    );
                    let blind = response.rect;
                    painter.rect_filled(blind, 6, colour(palette.background));
                    if let Some(texture) = texture {
                        let size = texture.size_vec2();
                        let left = match options.edge_side {
                            EdgeSide::Left => blind.left(),
                            EdgeSide::Right => blind.right() - size.x,
                        };
                        let top = blind.top()
                            + (blind.height() - size.y) * f32::from(options.edge_height_percent)
                                / 100.0;
                        let image = Rect::from_min_size(Pos2::new(left, top), size);
                        painter.image(
                            texture.id(),
                            image,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                }
            });
        } else {
            ui.weak(if options.edge {
                "No devices assigned to the edge slider."
            } else {
                "Edge slider is off."
            });
        }
    }

    fn save_settings(&mut self) {
        let mut updated = self.settings.clone().validated();
        if is_portable() {
            updated.start_with_windows = false;
        }
        let before = runtime::startup_enabled().unwrap_or(self.original.start_with_windows);
        match runtime::set_startup(updated.start_with_windows) {
            Ok(()) => match updated.save() {
                Ok(()) => {
                    if updated.battery_alerts_muted {
                        if let Some(notifier) = self.notifier.as_mut() {
                            notifier.clear();
                        }
                    }
                    self.original = updated.clone();
                    self.settings = updated;
                    self.message = Some((true, "Settings saved.".into()));
                    let _ = Log::Ui.event("settings", "Settings saved");
                }
                Err(error) => {
                    let rollback = runtime::set_startup(before);
                    self.message = Some((
                        false,
                        format!(
                            "Could not save settings: {error}{}",
                            if rollback.is_err() {
                                " (startup change could not be reverted)"
                            } else {
                                ""
                            }
                        ),
                    ));
                }
            },
            Err(error) => {
                self.message = Some((false, format!("Could not change startup: {error}")))
            }
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            ui.label(heading("Settings"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.settings != self.original,
                        egui::Button::new("Save settings"),
                    )
                    .clicked()
                {
                    self.save_settings();
                }
            });
        });
        if self.settings != self.original {
            ui.label(egui::RichText::new("Unsaved changes").small().weak());
        }
        if let Some((success, message)) = &self.message {
            let palette = self.palette(ui);
            ui.label(egui::RichText::new(message).color(colour(if *success {
                palette.full
            } else {
                palette.low
            })));
        }
        ui.add_space(12.0);
        ui.label(heading("Appearance"));
        egui::ComboBox::from_label("Theme")
            .selected_text(self.settings.visual_theme.label())
            .show_ui(ui, |ui| {
                for theme in VisualTheme::ALL {
                    let palette = theme.palette(theme.is_light(runtime::system_light_theme()));
                    ui.horizontal(|ui| {
                        ui.colored_label(colour(palette.accent), "●");
                        ui.selectable_value(&mut self.settings.visual_theme, theme, theme.label());
                    });
                }
            });
        if matches!(
            self.settings.visual_theme,
            VisualTheme::Ion | VisualTheme::Glass
        ) {
            egui::ComboBox::from_label("Mode")
                .selected_text(self.settings.theme.label())
                .show_ui(ui, |ui| {
                    for theme in Theme::ALL {
                        ui.selectable_value(&mut self.settings.theme, theme, theme.label());
                    }
                });
        }
        let mut transparency = self.settings.current_theme_transparency();
        if ui
            .add(
                egui::Slider::new(&mut transparency, 0..=85)
                    .suffix("%")
                    .text("Theme transparency"),
            )
            .changed()
        {
            self.settings
                .theme_transparency
                .insert(self.settings.visual_theme, transparency);
        }
        if transparency > 0
            && self.settings.visual_theme == VisualTheme::Glass
            && !self.glass_active
            && !self.motion.as_ref().is_some_and(WindowMotion::running)
        {
            ui.label(
                egui::RichText::new("Live blur is unavailable on this Windows setup.")
                    .small()
                    .weak(),
            );
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Choose Devices background").clicked() {
                match background::choose(frame) {
                    Ok(Some(id)) => {
                        self.settings.devices_background = Some(id);
                        self.message = None;
                        ui.ctx().request_repaint();
                    }
                    Ok(None) => {}
                    Err(error) => self.message = Some((false, error.to_string())),
                }
            }
            if self.settings.devices_background.is_some() && ui.button("Remove").clicked() {
                self.settings.devices_background = None;
                self.message = None;
                ui.ctx().request_repaint();
            }
        });
        if self.settings.devices_background.is_some() {
            ui.add(
                egui::Slider::new(&mut self.settings.background_opacity, 0..=100)
                    .suffix("%")
                    .text("Image opacity"),
            );
            if let Some(texture) = &self.background_texture {
                let (preview, painter) = ui
                    .allocate_painter(Vec2::new(ui.available_width(), 86.0), egui::Sense::hover());
                let base = colour(self.palette(ui).background);
                let rect = preview.rect;
                painter.rect_filled(rect, 10, base);
                let tint = self.background_tint();
                paint_workspace(&painter, rect, tint, tint, Some(texture));
            }
            if ui.small_button("Preview in Devices").clicked() {
                self.page = Page::Overview;
                ui.ctx().request_repaint();
            }
        }
        ui.add_space(7.0);
        ui.label(heading("General"));
        if is_portable() {
            self.settings.start_with_windows = false;
        }
        ui.add_enabled(
            !is_portable(),
            egui::Checkbox::new(&mut self.settings.start_with_windows, "Start with Windows"),
        )
        .on_disabled_hover_text("Available in the installed edition.");
        ui.add(
            egui::Slider::new(&mut self.settings.refresh_seconds, 15..=300)
                .suffix(" s")
                .text("Refresh interval"),
        );
        ui.add_space(9.0);
        ui.collapsing(
            egui::RichText::new("Device profiles").family(heading_family()),
            |ui| {
                let cards = self.device_cards(self.palette(ui), &current_snapshot(), unix_now());
                if cards.is_empty() {
                    ui.label("Connect a device to see its profile here.");
                }
                for card in cards {
                    let identity = card.profile.id().to_owned();
                    egui::CollapsingHeader::new(&card.name)
                        .id_salt(&identity)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} · {}",
                                    card.kind.label(),
                                    card.connection.label()
                                ))
                                .small()
                                .weak(),
                            );
                            let mut placement = self.settings.placement(&identity);
                            let original_placement = placement;
                            ui.checkbox(&mut placement.devices, "Show in Devices");
                            let has_battery = card.level.is_some();
                            let mut taskbar = placement.taskbar.unwrap_or(has_battery);
                            let mut edge = placement.edge.unwrap_or(has_battery);
                            if ui
                                .checkbox(&mut taskbar, "Show on taskbar indicator")
                                .changed()
                            {
                                placement.taskbar = Some(taskbar);
                            }
                            if ui.checkbox(&mut edge, "Show in edge slider").changed() {
                                placement.edge = Some(edge);
                            }
                            ui.checkbox(&mut placement.pinned, "Pin to top");
                            if placement != original_placement {
                                self.settings.set_placement(identity, placement);
                            }
                            match card.profile {
                                ProfileKey::BuiltIn(kind) => {
                                    let (visible, name) = match kind {
                                        DeviceKind::Headset => (
                                            &mut self.settings.show_headset,
                                            &mut self.settings.headset_name,
                                        ),
                                        DeviceKind::Mouse => (
                                            &mut self.settings.show_mouse,
                                            &mut self.settings.mouse_name,
                                        ),
                                    };
                                    ui.checkbox(visible, "Show device");
                                    ui.add(
                                        egui::TextEdit::singleline(name)
                                            .hint_text("Optional display name")
                                            .char_limit(30),
                                    );
                                    let profile = &mut self.settings.profiles[kind.index()];
                                    ui.checkbox(&mut profile.custom, "Custom battery settings");
                                    ui.add_enabled_ui(profile.custom, |ui| {
                                        ui.add(
                                            egui::Slider::new(
                                                &mut profile.refresh_seconds,
                                                15..=300,
                                            )
                                            .suffix(" s")
                                            .text("Refresh"),
                                        );
                                        ui.checkbox(
                                            &mut profile.low_notifications,
                                            "Low battery alert",
                                        );
                                        ui.add_enabled(
                                            profile.low_notifications,
                                            egui::Slider::new(&mut profile.low_threshold, 5..=50)
                                                .suffix("%")
                                                .text("Low threshold"),
                                        );
                                        if kind == DeviceKind::Mouse {
                                            ui.checkbox(
                                                &mut profile.full_notifications,
                                                "Full battery alert",
                                            );
                                        }
                                    });
                                }
                                ProfileKey::Detected(id) => {
                                    let original = self
                                        .settings
                                        .detected_profiles
                                        .get(&id)
                                        .cloned()
                                        .unwrap_or_default();
                                    let mut edited = original.clone();
                                    ui.checkbox(&mut edited.visible, "Show device");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut edited.name)
                                            .hint_text("Optional display name")
                                            .char_limit(30),
                                    );
                                    if edited != original {
                                        self.settings.detected_profiles.insert(id, edited);
                                    }
                                }
                            }
                        });
                }
            },
        );
        ui.collapsing(
            egui::RichText::new("Slider and taskbar").family(heading_family()),
            |ui| {
                let readings =
                    taskbar::widget_readings(&self.settings, &current_snapshot(), unix_now());
                let selected_name =
                    self.settings
                        .tray_device
                        .as_deref()
                        .map_or("Lowest battery", |id| {
                            readings
                                .iter()
                                .find(|reading| reading.id == id)
                                .map(|reading| reading.name.as_str())
                                .unwrap_or("Selected device unavailable")
                        });
                egui::ComboBox::from_label("Tray icon battery")
                    .selected_text(selected_name.to_owned())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.settings.tray_device, None, "Lowest battery");
                        for reading in readings.iter().filter(|reading| !reading.unavailable) {
                            ui.selectable_value(
                                &mut self.settings.tray_device,
                                Some(reading.id.clone()),
                                &reading.name,
                            );
                        }
                    });
                egui::ComboBox::from_label("Edge slider accent")
                    .selected_text(self.settings.widget_accent.label())
                    .show_ui(ui, |ui| {
                        for accent in Accent::ALL {
                            ui.selectable_value(
                                &mut self.settings.widget_accent,
                                accent,
                                accent.label(),
                            );
                        }
                    });
                ui.checkbox(
                    &mut self.settings.taskbar_widget,
                    "Show battery indicator on the taskbar",
                );
                ui.add(
                    egui::Slider::new(&mut self.settings.widget_scale, 80..=130)
                        .suffix("%")
                        .text("Widget size"),
                );
                let secondary = crate::taskbar::secondary_count();
                let selected = self.settings.monitor_slot;
                let label = if selected == 0 {
                    "Primary taskbar".to_owned()
                } else if selected <= secondary {
                    format!("Secondary taskbar {selected}")
                } else {
                    format!("Secondary taskbar {selected} (unavailable)")
                };
                egui::ComboBox::from_label("Widget monitor")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.settings.monitor_slot, 0, "Primary taskbar");
                        for slot in 1..=secondary {
                            ui.selectable_value(
                                &mut self.settings.monitor_slot,
                                slot,
                                format!("Secondary taskbar {slot}"),
                            );
                        }
                    });
                ui.add(
                    egui::Slider::new(&mut self.settings.widget_offset, 0..=240)
                        .suffix(" px")
                        .text("Left offset"),
                );
                ui.checkbox(
                    &mut self.settings.animate_charging,
                    "Animate charging indicator",
                );
                ui.checkbox(
                    &mut self.settings.edge_widget,
                    "Show screen-edge battery slider",
                );
                ui.add_enabled_ui(self.settings.edge_widget, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Screen edge");
                        ui.selectable_value(&mut self.settings.edge_side, EdgeSide::Left, "Left");
                        ui.selectable_value(&mut self.settings.edge_side, EdgeSide::Right, "Right");
                    });
                    ui.add(
                        egui::Slider::new(&mut self.settings.edge_height_percent, 0..=100)
                            .suffix("%")
                            .text("Position from top"),
                    );
                });
                self.widget_preview(ui, &readings);
            },
        );
        ui.collapsing(egui::RichText::new("Notifications").family(heading_family()), |ui| {
            ui.checkbox(&mut self.settings.battery_alerts_muted, "Mute battery and connection alerts");
            ui.checkbox(&mut self.settings.low_notifications, "Notify when battery is low");
            ui.add_enabled(self.settings.low_notifications, egui::Slider::new(&mut self.settings.low_threshold, 5..=50).suffix("%").text("Low battery"));
            ui.checkbox(&mut self.settings.full_notifications, "Notify when charging is complete");
            ui.checkbox(&mut self.settings.connection_alerts, "Show battery when a supported device reconnects");
            ui.checkbox(&mut self.settings.notification_sound, "Play notification sound (Windows settings apply)");
            let muted = self.original.battery_alerts_muted ||
                TemporaryMute::load().active(unix_now(), &current_snapshot().monitor_session);
            if ui.add_enabled(!muted, egui::Button::new("Test notification")).clicked() {
                self.notifier.get_or_insert_with(runtime::Notifier::new).show("GearDot", "Test notification — your app is ready.", true, self.settings.notification_sound);
                let _ = Log::Ui.event("notification", "Test requested");
                self.message = Some((true, "Test requested. Check Windows notification settings if it does not appear.".into()));
            }
        });
        ui.collapsing(
            egui::RichText::new("Backup and diagnostics").family(heading_family()),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Export saved settings").clicked() {
                        self.message = Some(match self.original.backup() {
                            Ok(()) => (
                                true,
                                "Saved settings-backup.json in the data folder.".into(),
                            ),
                            Err(e) => (false, format!("Backup failed: {e}")),
                        });
                    }
                    if ui.button("Load settings backup").clicked() {
                        match Settings::restore_backup() {
                            Ok(mut saved) => {
                                // Loading a backup never changes the actual startup registration.
                                saved.start_with_windows = self.settings.start_with_windows;
                                self.settings = saved;
                                self.message =
                                    Some((true, "Backup loaded. Click Save to apply.".into()));
                            }
                            Err(e) => self.message = Some((false, format!("Restore failed: {e}"))),
                        }
                    }
                });
                if ui.button("Open data and logs folder").clicked() {
                    let result = data_dir().and_then(|dir| {
                        std::fs::create_dir_all(&dir)?;
                        std::process::Command::new("explorer.exe")
                            .arg(dir)
                            .spawn()
                            .map(|_| ())
                    });
                    if let Err(e) = result {
                        self.message = Some((false, format!("Cannot open folder: {e}")));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Reset defaults").clicked() {
                        self.settings = Settings::default();
                        self.message = Some((true, "Defaults loaded. Click Save to apply.".into()));
                    }
                });
            },
        );
    }

    fn about_page(&self, ui: &mut egui::Ui) {
        const PROJECT_URL: &str = "https://github.com/SymbianDevAhmed/GearDot";
        let palette = self.palette(ui);
        let muted = colour(palette.muted);
        let accent = colour(palette.accent);
        let surface = colour(palette.surface);
        let section = |title| heading(title).size(18.0).color(accent);

        ui.label(heading("About GearDot").size(24.0));
        ui.add_space(8.0);
        egui::Frame::new().fill(surface).corner_radius(15).inner_margin(14).show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(texture) = &self.icon_texture {
                    ui.image((texture.id(), Vec2::splat(54.0)));
                }
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.label(heading("GearDot").size(30.0).color(accent));
                    ui.label(egui::RichText::new("Peripheral battery monitor").size(15.0).color(muted));
                });
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new("See your connected devices and their available battery levels from the Windows tray.").size(15.0));
        });

        ui.add_space(13.0);
        ui.label(section("App information"));
        egui::Frame::new()
            .fill(surface)
            .corner_radius(13)
            .inner_margin(13)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let edition = if is_portable() {
                    "Portable"
                } else {
                    "Installed / standalone"
                };
                for (label, value) in [
                    ("Version", env!("CARGO_PKG_VERSION")),
                    ("Platform", "Windows x64"),
                    ("Edition", edition),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(label).size(14.0).color(muted));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(value)
                                    .size(14.0)
                                    .family(heading_family()),
                            );
                        });
                    });
                }
            });

        ui.add_space(13.0);
        ui.label(section("Project links"));
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(egui::RichText::new("Documentation").size(14.0), PROJECT_URL);
            ui.label(egui::RichText::new("·").color(muted));
            ui.hyperlink_to(
                egui::RichText::new("Check for updates").size(14.0),
                format!("{PROJECT_URL}/releases"),
            );
        });

        ui.add_space(13.0);
        ui.label(
            egui::RichText::new("Unofficial project; not affiliated with Logitech, VXE or ATK.")
                .size(13.0)
                .color(muted),
        );
        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("© 2026 Ahmed Gaber")
                    .size(14.0)
                    .family(heading_family()),
            );
            ui.hyperlink_to(
                egui::RichText::new("@SymbianDevAhmed").size(14.0),
                "https://github.com/SymbianDevAhmed",
            );
        });
    }
}

impl eframe::App for DetailsCard {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.icon_texture.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [64, 64],
                include_bytes!(concat!(env!("OUT_DIR"), "/app-icon.rgba")),
            );
            self.icon_texture =
                Some(ctx.load_texture("app-emblem", image, egui::TextureOptions::LINEAR));
        }
        self.window_icons.refresh(frame);
        let now = Instant::now();
        let motion = self.motion.get_or_insert_with(|| {
            if self.animations {
                WindowMotion::Opening(now)
            } else {
                WindowMotion::Still
            }
        });
        if matches!(motion, WindowMotion::Closed) {
            return;
        }
        if self.animations && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            motion.close(now);
        }
        if self.popup && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.request_close(ctx, now);
        }
        let motion = self.motion.as_mut().expect("window motion initialized");
        motion.advance(now);
        if matches!(motion, WindowMotion::Closed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let scale = motion.scale(now);
        if motion.running() {
            ctx.request_repaint();
        }

        let mode_light = match self.settings.theme {
            Theme::System => runtime::system_light_theme(),
            Theme::Light => true,
            Theme::Dark => false,
        };
        let theme = self.settings.visual_theme;
        let light = theme.is_light(mode_light);
        let transparency = self.settings.current_theme_transparency();
        let wants_glass = theme == VisualTheme::Glass
            && transparency > 0
            && !motion.running()
            && runtime::transparency_enabled();
        if let Some(hwnd) = runtime::details_hwnd(frame) {
            runtime::keep_details_frameless(hwnd);
            if self.backdrop != Some((wants_glass, light, transparency)) {
                self.glass_active =
                    runtime::set_composition(hwnd, wants_glass, light, transparency);
                self.backdrop = Some((wants_glass, light, transparency));
            }
        }

        let palette = theme.palette(light);
        let background = colour(palette.background);
        let glass = self.glass_active;
        let opacity = 1.0 - f32::from(transparency) / 100.0;
        let root_colour = if theme == VisualTheme::Glass && !glass {
            background.lerp_to_gamma(colour(palette.surface), 0.45)
        } else {
            background
        };
        let root_fill = root_colour.gamma_multiply(opacity);
        ctx.set_visuals(themed_visuals(palette, light));
        let window = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());

        let workspace = Rect::from_min_max(
            window.min + Vec2::new(NAVIGATION_WIDTH - 1.0, 43.0),
            window.max - Vec2::splat(8.0),
        );
        let surface = colour(palette.surface);
        let raised = background.lerp_to_gamma(surface, 0.58);
        let top = if theme == VisualTheme::Glass {
            raised.lerp_to_gamma(colour(palette.accent), 0.13)
        } else {
            raised.lerp_to_gamma(surface, 0.23)
        };
        let top = top.gamma_multiply(opacity);
        let bottom = raised.gamma_multiply(if glass { opacity * 0.7 } else { opacity });
        // Draw each surface once so their alpha does not accumulate across the workspace.
        paint_window_chrome(&painter, window, workspace, root_fill);
        paint_workspace(&painter, workspace, top, bottom, None);
        if self.background_id != self.settings.devices_background {
            self.background_id
                .clone_from(&self.settings.devices_background);
            self.background_texture =
                self.background_id
                    .as_deref()
                    .and_then(|id| match background::texture(ctx, id) {
                        Ok(texture) => Some(texture),
                        Err(error) => {
                            self.message = Some((
                                false,
                                format!("Could not load the Devices background: {error}"),
                            ));
                            None
                        }
                    });
        }
        if self.page == Page::Overview {
            if let Some(texture) = &self.background_texture {
                let tint = self.background_tint();
                paint_workspace(&painter, workspace, tint, tint, Some(texture));
            }
        }
        if theme == VisualTheme::Glass {
            paint_glass_edge(&painter, window.shrink(0.7), light);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        if let Some(notifier) = self.notifier.as_mut() {
            notifier.tick();
        }

        egui::TopBottomPanel::top("title")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 7)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (title, painter) = ui.allocate_painter(
                        Vec2::new((ui.available_width() - 76.0).max(48.0), 24.0),
                        egui::Sense::click_and_drag(),
                    );
                    let start = title.rect.left_center();
                    if let Some(texture) = &self.icon_texture {
                        let emblem =
                            Rect::from_center_size(start + Vec2::new(11.0, 0.0), Vec2::splat(22.0));
                        painter.image(
                            texture.id(),
                            emblem,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    painter.text(
                        start + Vec2::new(29.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        "GearDot",
                        egui::FontId::new(19.0, heading_family()),
                        colour(palette.accent),
                    );
                    if title.drag_started_by(egui::PointerButton::Primary) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").on_hover_text("Close").clicked() {
                            self.request_close(ctx, now);
                        }
                        if ui.button("−").on_hover_text("Minimize").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    });
                });
            });

        egui::SidePanel::left("navigation")
            .show_separator_line(false)
            .exact_width(NAVIGATION_WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(NAVIGATION_PADDING, 14)))
            .show(ctx, |ui| {
                if navigation_item(
                    ui,
                    palette,
                    Page::Overview,
                    "Devices",
                    self.page == Page::Overview,
                ) {
                    self.page = Page::Overview;
                }
                if self.demo {
                    return;
                }
                for (page, label) in [
                    (Page::QuickControls, "Controls"),
                    (Page::Diagnostics, "Diagnostics"),
                ] {
                    if navigation_item(ui, palette, page, label, self.page == page) {
                        self.page = page;
                        self.message = None;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    for (page, label) in [(Page::About, "About"), (Page::Settings, "Settings")] {
                        if navigation_item(ui, palette, page, label, self.page == page) {
                            self.page = page;
                        }
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 14,
                right: 20,
                top: 15,
                bottom: 16,
            }))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::Overview => self.overview(ui),
                    Page::QuickControls => self.quick_controls_page(ui),
                    Page::Diagnostics => self.diagnostics_page(ui),
                    Page::Settings => self.settings_page(ui, frame),
                    Page::About => self.about_page(ui),
                });
            });
        if scale != 1.0 {
            let translation = window.center().to_vec2() * (1.0 - scale);
            ctx.transform_layer_shapes(
                egui::LayerId::background(),
                egui::emath::TSTransform::new(translation, scale),
            );
        }
    }
}

fn paint_glass_edge(painter: &egui::Painter, bounds: Rect, light: bool) {
    let highlight = |alpha| Color32::from_white_alpha(alpha);
    painter.rect_stroke(
        bounds,
        WINDOW_RADIUS,
        Stroke::new(1.0_f32, highlight(if light { 145 } else { 88 })),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        bounds.shrink(1.2),
        WINDOW_RADIUS - 1.2,
        Stroke::new(2.0_f32, highlight(25)),
        egui::StrokeKind::Inside,
    );
    let y = bounds.top() + 1.8;
    let edge = |fraction| Pos2::new(egui::lerp(bounds.x_range(), fraction), y);
    painter.line_segment(
        [edge(0.075), edge(0.36)],
        Stroke::new(1.0_f32, highlight(105)),
    );
    painter.line_segment(
        [edge(0.36), edge(0.55)],
        Stroke::new(1.0_f32, highlight(45)),
    );
}

fn rounded_outline(rect: Rect, radius: f32) -> impl Iterator<Item = Pos2> {
    let radius = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let centers = [
        rect.right_top() + Vec2::new(-radius, radius),
        rect.right_bottom() - Vec2::splat(radius),
        rect.left_bottom() + Vec2::new(radius, -radius),
        rect.left_top() + Vec2::splat(radius),
    ];
    centers
        .into_iter()
        .enumerate()
        .flat_map(move |(corner, center)| {
            (0..=CORNER_STEPS).map(move |step| {
                let angle = (corner as f32 - 1.0 + step as f32 / CORNER_STEPS as f32)
                    * std::f32::consts::FRAC_PI_2;
                center + Vec2::new(angle.cos(), angle.sin()) * radius
            })
        })
}

fn paint_window_chrome(painter: &egui::Painter, window: Rect, workspace: Rect, fill: Color32) {
    let feather = 1.0 / painter.ctx().pixels_per_point();
    let inner_edge = rounded_outline(window.shrink(feather), WINDOW_RADIUS - feather);
    let outer_edge = rounded_outline(window, WINDOW_RADIUS);
    let workspace_edge = rounded_outline(workspace, WORKSPACE_RADIUS);
    let mut mesh = egui::Mesh::default();
    for ((outer, inner), workspace) in outer_edge.zip(inner_edge).zip(workspace_edge) {
        mesh.colored_vertex(outer, Color32::TRANSPARENT);
        mesh.colored_vertex(inner, fill);
        mesh.colored_vertex(workspace, fill);
    }
    let vertex_count = mesh.vertices.len() as u32;
    for i in (0..vertex_count).step_by(3) {
        let next = (i + 3) % vertex_count;
        mesh.add_triangle(i, next, next + 1);
        mesh.add_triangle(i, next + 1, i + 1);
        mesh.add_triangle(i + 1, next + 1, next + 2);
        mesh.add_triangle(i + 1, next + 2, i + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_workspace(
    painter: &egui::Painter,
    rect: Rect,
    top: Color32,
    bottom: Color32,
    image: Option<&egui::TextureHandle>,
) {
    if !rect.is_positive() {
        return;
    }
    let tint = |position: Pos2| {
        top.lerp_to_gamma(
            bottom,
            ((position.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
        )
    };
    let mut mesh = image.map_or_else(egui::Mesh::default, |image| {
        egui::Mesh::with_texture(image.id())
    });
    let crop = image.map(|image| {
        let size = image.size_vec2();
        let scale = (rect.width() / size.x).max(rect.height() / size.y);
        rect.size() / (size * scale)
    });
    let vertex = |mesh: &mut egui::Mesh, position: Pos2| {
        if let Some(crop) = crop {
            let relative = (position - rect.center()) / rect.size();
            let uv = Pos2::new(0.5 + relative.x * crop.x, 0.5 + relative.y * crop.y);
            mesh.vertices.push(egui::epaint::Vertex {
                pos: position,
                uv,
                color: tint(position),
            });
        } else {
            mesh.colored_vertex(position, tint(position));
        }
    };
    vertex(&mut mesh, rect.center());
    for position in rounded_outline(rect, WORKSPACE_RADIUS) {
        vertex(&mut mesh, position);
    }
    let last = (mesh.vertices.len() - 1) as u32;
    for index in 1..last {
        mesh.add_triangle(0, index, index + 1);
    }
    mesh.add_triangle(0, last, 1);
    painter.add(egui::Shape::mesh(mesh));
}

fn navigation_item(
    ui: &mut egui::Ui,
    palette: &Palette,
    page: Page,
    label: &str,
    selected: bool,
) -> bool {
    let (response, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), 48.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    let bounds = response.rect;
    if selected || response.hovered() || response.has_focus() {
        painter.rect_filled(bounds, 7, colour(palette.selection));
    }
    let ink = colour(if selected {
        palette.accent
    } else {
        palette.text
    });
    let center = Pos2::new(bounds.center().x, bounds.top() + 16.0);
    let p = |x, y| center + Vec2::new(x, y);
    let stroke = Stroke::new(1.3_f32, ink);
    match page {
        Page::Overview => {
            painter.rect_stroke(
                Rect::from_min_max(p(-7.0, -5.0), p(5.0, 5.0)),
                2,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([p(6.0, -2.0), p(6.0, 2.0)], stroke);
        }
        Page::QuickControls => {
            painter.add(egui::Shape::line(
                vec![
                    p(1.0, -7.0),
                    p(-5.0, 1.0),
                    p(0.0, 1.0),
                    p(-1.0, 7.0),
                    p(5.0, -1.0),
                    p(0.0, -1.0),
                    p(1.0, -7.0),
                ],
                stroke,
            ));
        }
        Page::Diagnostics => {
            painter.circle_stroke(p(-1.5, -1.5), 4.5, stroke);
            painter.line_segment([p(2.0, 2.0), p(6.0, 6.0)], stroke);
        }
        Page::Settings => {
            for (y, x) in [(-4.5, -2.5), (0.0, 2.0), (4.5, -1.0)] {
                painter.line_segment([p(-7.0, y), p(7.0, y)], stroke);
                painter.circle_filled(p(x, y), 2.1, ink);
            }
        }
        Page::About => {
            painter.circle_stroke(center, 7.0, stroke);
            painter.circle_filled(p(0.0, -3.0), 0.9, ink);
            painter.line_segment([p(0.0, 0.0), p(0.0, 3.5)], stroke);
        }
    }
    let label_bounds = bounds.shrink2(Vec2::new(5.0, 0.0));
    let font = egui::FontId::proportional(11.5);
    let mut text = painter.layout_no_wrap(label.to_owned(), font.clone(), ink);
    if text.size().x > label_bounds.width() {
        let scale = label_bounds.width() / text.size().x;
        text = painter.layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(font.size * scale),
            ink,
        );
    }
    let label_center = Pos2::new(bounds.center().x, bounds.top() + 34.0);
    painter
        .with_clip_rect(label_bounds)
        .galley(label_center - text.size() / 2.0, text, ink);
    response.clicked()
}
