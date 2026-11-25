use crate::{icon::BatteryIcon, runtime::Notifier, taskbar::{TaskbarWidget, WidgetEvent, WidgetReading}};
use wireless_battery_monitor::{
    battery::{BatteryError, BatteryReading, BatterySource, DeviceKind},
    history::BatteryHistory,
    mouse::MadRReceiver,
    preferences::{Alert, AlertTracker, LiveReading, LiveSnapshot, Settings},
    receiver::G435Receiver,
};
use std::time::{Duration, Instant};
use tray_icon::{menu::{Menu, MenuEvent, MenuItem}, TrayIcon, TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState};
use winit::{
    application::ApplicationHandler, event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy}, window::WindowId,
};

const SETTINGS_CHECK: Duration = Duration::from_secs(2);
const RECONNECT: Duration = Duration::from_secs(15);

enum Event {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Widget(WidgetEvent),
    Reading(DeviceKind, Result<BatteryReading, BatteryError>),
}

/// Presentation and device schedules are independent of battery protocols.
pub struct TrayApp {
    tray: Option<TrayIcon>,
    widget: Option<TaskbarWidget>,
    notifier: Option<Notifier>,
    status: [MenuItem; 2],
    refresh: MenuItem,
    details: MenuItem,
    settings_item: MenuItem,
    histories: [BatteryHistory; 2],
    alerts: [AlertTracker; 2],
    snapshot: LiveSnapshot,
    settings: Settings,
    details_process: Option<std::process::Child>,
    quit: MenuItem,
    proxy: EventLoopProxy<Event>,
    pending: [bool; 2],
    quitting: bool,
    next_due: [Instant; 2],
    next_settings: Instant,
    startup_error: Option<String>,
}

impl TrayApp {
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let event_loop = EventLoop::<Event>::with_user_event().build()?;
        let proxy = event_loop.create_proxy();
        let menu_proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |event| { let _ = menu_proxy.send_event(Event::Menu(event)); }));
        let tray_proxy = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |event| { let _ = tray_proxy.send_event(Event::Tray(event)); }));
        let settings = Settings::load();
        let snapshot = LiveSnapshot { refresh_seconds: settings.refresh_seconds, ..Default::default() };
        let _ = snapshot.save();
        let now = Instant::now();
        let mut app = Self {
            tray: None, widget: None, notifier: None,
            status: [MenuItem::new("Checking G435...", false, None), MenuItem::new("Checking MAD R...", false, None)],
            refresh: MenuItem::new("Refresh now", true, None),
            details: MenuItem::new("Battery details", true, None),
            settings_item: MenuItem::new("Settings", true, None),
            histories: DeviceKind::ALL.map(BatteryHistory::load),
            alerts: std::array::from_fn(|_| AlertTracker::default()),
            snapshot, settings,
            details_process: None, quit: MenuItem::new("Exit", true, None), proxy,
            pending: [false; 2], quitting: false, next_due: [now; 2], next_settings: now,
            startup_error: None,
        };
        event_loop.run_app(&mut app)?;
        for reading in &mut app.snapshot.devices {
            reading.stale = true; reading.checked_at = 0; reading.message = "Monitor exited.".into();
        }
        let _ = app.snapshot.save();
        if let Some(error) = app.startup_error { return Err(std::io::Error::other(error).into()); }
        Ok(())
    }

    fn create_tray(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let menu = Menu::new();
        for item in &self.status { menu.append(item)?; }
        menu.append(&self.details)?;
        menu.append(&self.settings_item)?;
        menu.append(&self.refresh)?;
        menu.append(&self.quit)?;
        self.tray = Some(TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Wireless Battery Monitor: checking...")
            .with_menu_on_left_click(false)
            .with_icon(BatteryIcon::render(None)?)
            .build()?);
        let proxy = self.proxy.clone();
        self.widget = TaskbarWidget::new(move |event| { let _ = proxy.send_event(Event::Widget(event)); });
        self.notifier = Some(Notifier::new());
        Ok(())
    }

    fn open_details(&mut self, settings: bool) {
        if let Some(child) = self.details_process.as_mut() {
            if matches!(child.try_wait(), Ok(None)) { crate::runtime::focus_details(); return; }
        }
        if let Ok(exe) = std::env::current_exe() {
            self.details_process = std::process::Command::new(exe)
                .arg(if settings { "--settings" } else { "--details" }).spawn().ok();
        }
    }

    fn request(&mut self, kind: DeviceKind) {
        let index = kind.index();
        if self.pending[index] || self.quitting { return; }
        self.pending[index] = true;
        self.refresh.set_enabled(false);
        let proxy = self.proxy.clone();
        // Distinct devices do not delay one another while disconnected or asleep.
        std::thread::spawn(move || {
            let reading = match kind {
                DeviceKind::Headset => G435Receiver::new().read_battery(),
                DeviceKind::Mouse => MadRReceiver.read_battery(),
            };
            let _ = proxy.send_event(Event::Reading(kind, reading));
        });
    }

    fn refresh_now(&mut self) {
        for kind in DeviceKind::ALL { self.request(kind); }
    }

    fn refresh_settings(&mut self, now: Instant) {
        self.next_settings = now + SETTINGS_CHECK;
        if let Some(notifier) = self.notifier.as_mut() { notifier.tick(); }
        let updated = Settings::load();
        if updated != self.settings {
            let interval_changed = updated.refresh_seconds != self.settings.refresh_seconds;
            self.settings = updated;
            self.snapshot.refresh_seconds = self.settings.refresh_seconds;
            let _ = self.snapshot.save();
            if interval_changed {
                for (i, due) in self.next_due.iter_mut().enumerate() {
                    if !self.pending[i] { *due = now; }
                }
            }
            self.update_ui();
        }
    }

    fn apply_reading(&mut self, kind: DeviceKind, result: Result<BatteryReading, BatteryError>) {
        let index = kind.index();
        self.pending[index] = false;
        self.refresh.set_enabled(!self.pending.iter().any(|v| *v));
        let now = BatteryHistory::now();
        let history = &mut self.histories[index];
        let mut delay = Duration::from_secs(self.settings.refresh_seconds);
        let status = match result {
            Ok(reading) => {
                let _ = history.record(reading);
                if let Some(alert) = self.alerts[index].observe(reading, &self.settings) {
                    if let Some(notifier) = self.notifier.as_mut() {
                        match alert {
                            Alert::Low => notifier.show("Low battery", &format!("{}: {}{}%. Time to charge.",
                                kind.name(), if reading.estimated() { "~" } else { "" }, reading.percentage()), false),
                            Alert::Full => notifier.show("Battery full", &format!("{} is fully charged (100%).", kind.name()), true),
                        }
                    }
                }
                // Charge progress is refreshed at least every 15 seconds.
                if reading.charging() == Some(true) && reading.percentage() < 100 {
                    delay = delay.min(RECONNECT);
                }
                LiveReading::from_reading(reading, now)
            },
            Err(error) => {
                delay = delay.min(RECONNECT);
                if matches!(error, BatteryError::ReceiverNotFound) { self.alerts[index].disconnected(); }
                let mut status = LiveReading { checked_at: now, message: error.to_string(),
                    estimated: kind == DeviceKind::Headset, ..Default::default() };
                if kind == DeviceKind::Mouse && !matches!(error, BatteryError::ReceiverNotFound) {
                    if let Some(previous) = history.latest().filter(|s| now.saturating_sub(s.at) <= 1800) {
                        status.level = Some(previous.percent);
                        status.millivolts = Some(previous.millivolts);
                        status.stale = true;
                        if now.saturating_sub(previous.at) > 300 { self.alerts[index].disconnected(); }
                    } else { self.alerts[index].disconnected(); }
                }
                status
            },
        };
        self.next_due[index] = Instant::now() + delay;
        self.snapshot.devices[index] = status;
        let _ = self.snapshot.save();
        self.update_ui();
    }

    fn update_ui(&mut self) {
        let readings = self.snapshot.devices.each_ref().map(|reading| WidgetReading {
            level: reading.level, charging: reading.charging == Some(true), stale: reading.stale,
            full: reading.confirmed_full(), low_threshold: self.settings.low_threshold,
        });
        let mut messages = Vec::new();
        for kind in DeviceKind::ALL {
            let reading = &self.snapshot.devices[kind.index()];
            let message = if let Some(level) = reading.level {
                format!("{}: {}{}% — {}", kind.name(), if reading.estimated || reading.stale { "~" } else { "" },
                    level, reading.state_label())
            } else {
                format!("{}: {}", kind.name(), if reading.message.is_empty() { "Checking..." } else { &reading.message })
            };
            self.status[kind.index()].set_text(&message);
            messages.push(message);
        }
        if let Some(widget) = self.widget.as_mut() { widget.set_readings(readings); }
        if let Some(tray) = &self.tray {
            if let Ok(icon) = BatteryIcon::render(readings[0].level.or(readings[1].level)) { let _ = tray.set_icon(Some(icon)); }
            let tooltip: String = messages.join("\n").chars().take(120).collect();
            let _ = tray.set_tooltip(Some(&tooltip));
        }
    }

    fn exit(&mut self, event_loop: &ActiveEventLoop) {
        self.quitting = true;
        if !self.pending.iter().any(|v| *v) { event_loop.exit(); }
    }
}

impl ApplicationHandler<Event> for TrayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.tray.is_none() {
            if let Err(error) = self.create_tray() {
                self.startup_error = Some(error.to_string());
                event_loop.exit(); return;
            }
            self.refresh_now();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::Menu(event) if event.id() == self.quit.id() => self.exit(event_loop),
            Event::Menu(event) if event.id() == self.refresh.id() => self.refresh_now(),
            Event::Menu(event) if event.id() == self.details.id() => self.open_details(false),
            Event::Menu(event) if event.id() == self.settings_item.id() => self.open_details(true),
            Event::Menu(_) => {},
            Event::Tray(TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) => self.open_details(false),
            Event::Tray(_) => {},
            Event::Widget(WidgetEvent::Details) => self.open_details(false),
            Event::Widget(WidgetEvent::Settings) => self.open_details(true),
            Event::Widget(WidgetEvent::Refresh) => self.refresh_now(),
            Event::Widget(WidgetEvent::Exit) => self.exit(event_loop),
            Event::Widget(WidgetEvent::Visibility(visible)) => {
                if let Some(tray) = &self.tray { let _ = tray.set_visible(!visible); }
            },
            Event::Reading(kind, result) => {
                self.apply_reading(kind, result);
                if self.quitting && !self.pending.iter().any(|v| *v) { event_loop.exit(); }
            },
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_settings { self.refresh_settings(now); }
        let mut next = self.next_settings;
        for kind in DeviceKind::ALL {
            let index = kind.index();
            if !self.pending[index] && !self.quitting {
                if now >= self.next_due[index] { self.request(kind); }
                else { next = next.min(self.next_due[index]); }
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
