use crate::{bluetooth, icon::{BatteryIcon, DeviceGlyph}, runtime::Notifier, taskbar::{TaskbarWidget, WidgetEvent, WidgetReading}};
use wireless_battery_monitor::{
    battery::{BatteryError, BatteryReading, DeviceKind},
    devices, diagnostics::Log,
    history::BatteryHistory,
    preferences::{data_dir, Alert, AlertTracker, BluetoothReading, LiveReading, LiveSnapshot, Settings},
};
use std::time::{Duration, Instant};
use tray_icon::{menu::{Menu, MenuEvent, MenuItem}, TrayIcon, TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState};
use winit::{
    application::ApplicationHandler, event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy}, window::WindowId,
};

const SETTINGS_CHECK: Duration = Duration::from_secs(2);


enum Event {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Widget(WidgetEvent),
    Reading(DeviceKind, u64, Result<BatteryReading, BatteryError>),
    Bluetooth(u64, windows::core::Result<Vec<BluetoothReading>>),
}

/// Presentation and device schedules are independent of battery protocols.
pub struct TrayApp {
    tray: Option<TrayIcon>,
    widget: Option<TaskbarWidget>,
    edge: Option<TaskbarWidget>,
    bluetooth_failures: u8,
    bluetooth_pending: bool,
    next_bluetooth: Instant,
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
    suspended: bool,
    epoch: u64,
    last_wall_check: u64,
    next_due: [Instant; 2],
    next_settings: Instant,
    startup_error: Option<String>,
}

impl TrayApp {
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let _ = Log::Monitor.event("lifecycle", concat!("Started v", env!("CARGO_PKG_VERSION")));
        let event_loop = EventLoop::<Event>::with_user_event().build()?;
        let proxy = event_loop.create_proxy();
        let menu_proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |event| { let _ = menu_proxy.send_event(Event::Menu(event)); }));
        let tray_proxy = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |event| { let _ = tray_proxy.send_event(Event::Tray(event)); }));
        let settings = Settings::load();
        let snapshot = LiveSnapshot { refresh_seconds: settings.refresh_seconds, device_intervals: DeviceKind::ALL.map(|k| settings.poll_seconds(k)), ..Default::default() };
        let _ = snapshot.save();
        let now = Instant::now();
        let mut app = Self {
            tray: None, widget: None, edge: None, bluetooth_failures: 0, bluetooth_pending: false, next_bluetooth: now, notifier: None,
            status: [MenuItem::new("Checking G435...", false, None), MenuItem::new("Checking MAD R...", false, None)],
            refresh: MenuItem::new("Refresh now", true, None),
            details: MenuItem::new("Battery details", true, None),
            settings_item: MenuItem::new("Settings", true, None),
            histories: DeviceKind::ALL.map(BatteryHistory::load),
            alerts: std::array::from_fn(|_| AlertTracker::default()),
            snapshot, settings,
            details_process: None, quit: MenuItem::new("Exit", true, None), proxy,
            pending: [false; 2], quitting: false, suspended: false, epoch: 0, last_wall_check: BatteryHistory::now(), next_due: [now; 2], next_settings: now,
            startup_error: None,
        };
        event_loop.run_app(&mut app)?;
        for reading in &mut app.snapshot.devices {
            reading.stale = true; reading.checked_at = 0; reading.message = "Monitor exited.".into();
        }
        app.snapshot.bluetooth.clear();
        let _ = app.snapshot.save();
        let _ = Log::Monitor.event("lifecycle", "Exited");
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
        self.widget = TaskbarWidget::new(self.settings.widget_options(), false, move |event| { let _ = proxy.send_event(Event::Widget(event)); });
        let proxy = self.proxy.clone();
        self.edge = TaskbarWidget::new(self.settings.widget_options(), true, move |event| {
            let _ = proxy.send_event(Event::Widget(event));
        });
        self.notifier = Some(Notifier::new());
        self.update_ui();
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
        if self.pending[index] || self.quitting || self.suspended { return; }
        self.pending[index] = true;
        self.refresh.set_enabled(false);
        let proxy = self.proxy.clone();
        let epoch = self.epoch;
        // Distinct devices do not delay one another while disconnected or asleep.
        std::thread::spawn(move || {
            let reading = devices::read(kind);
            let _ = proxy.send_event(Event::Reading(kind, epoch, reading));
        });
    }

    fn request_bluetooth(&mut self) {
        if self.bluetooth_pending || self.suspended || self.quitting { return; }
        self.bluetooth_pending = true;
        let proxy = self.proxy.clone();
        let epoch = self.epoch;
        std::thread::spawn(move || {
            let _ = proxy.send_event(Event::Bluetooth(epoch, bluetooth::connected_readings()));
        });
    }

    fn refresh_now(&mut self) {
        for kind in DeviceKind::ALL { self.request(kind); }
        self.request_bluetooth();
    }

    fn refresh_settings(&mut self, now: Instant) {
        self.next_settings = now + if self.settings.power_saving { Duration::from_secs(5) } else { SETTINGS_CHECK };
        let wall = BatteryHistory::now();
        if wall.saturating_sub(self.last_wall_check) > 20 {
            self.resume_devices();
        }
        self.last_wall_check = wall;
        if let Some(notifier) = self.notifier.as_mut() { notifier.tick(); }
        if let Ok(dir) = data_dir() {
            if std::fs::remove_file(dir.join("refresh.request")).is_ok() {
                self.refresh_now();
            }
        }
        let updated = Settings::load();
        if updated != self.settings {
            let interval_changed = updated.refresh_seconds != self.settings.refresh_seconds || updated.profiles != self.settings.profiles || updated.power_saving != self.settings.power_saving;
            self.settings = updated;
            let _ = Log::Monitor.event("settings", "Preferences applied");
            self.snapshot.refresh_seconds = self.settings.refresh_seconds;
            self.snapshot.device_intervals = DeviceKind::ALL.map(|k| self.settings.poll_seconds(k));
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
        let profile = self.settings.profile(kind);
        let mut delay = Duration::from_secs(self.settings.poll_seconds(kind));
        let status = match result {
            Ok(reading) => {
                if history.record(reading).is_err() { let _ = Log::Monitor.event("storage", "Could not save history"); }
                if let Some(alert) = self.alerts[index].observe_profile(reading, profile) {
                    let _ = Log::Monitor.event("alert", &format!("{}: {:?}", kind.name(), alert));
                    if let Some(notifier) = self.notifier.as_mut() {
                        match alert {
                            Alert::Low => notifier.show("Low battery", &format!("{}: {}{}%. Time to charge.",
                                self.settings.device_name(kind), if reading.estimated() { "~" } else { "" }, reading.percentage()), false, self.settings.notification_sound),
                            Alert::Full => notifier.show("Battery full", &format!("{} is fully charged (100%).", self.settings.device_name(kind)), true, self.settings.notification_sound),
                        }
                    }
                }
                // Charge progress is refreshed at least every 15 seconds.
                if reading.charging() == Some(true) && reading.percentage() < 100 {
                    delay = delay.min(Duration::from_secs(self.settings.charge_seconds()));
                }
                LiveReading::from_reading(reading, now)
            },
            Err(error) => {
                delay = Duration::from_secs(self.settings.reconnect_seconds());
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
        let previous = &self.snapshot.devices[index];
        if self.settings.connection_alerts && previous.checked_at != 0 && !previous.message.is_empty()
            && !previous.online() && status.online() {
            if let (Some(level), Some(notifier)) = (status.level, self.notifier.as_mut()) {
                let name = self.settings.device_name(kind);
                let prefix = if status.estimated { "~" } else { "" };
                notifier.show("Device connected", &format!("{name} · {prefix}{level}%"), true,
                    self.settings.notification_sound);
            }
        }
        if previous.level.is_some() != status.level.is_some() || previous.stale != status.stale || previous.charging != status.charging || previous.message != status.message {
            let label = if status.level.is_none() { "Unavailable" } else { status.state_label() };
            let _ = Log::Monitor.event("device", &format!("{}: {}", kind.name(), label));
        }
        self.snapshot.devices[index] = status;
        let _ = self.snapshot.save();
        self.update_ui();
    }

    fn update_ui(&mut self) {
        let options = self.settings.widget_options();
        let mut readings = Vec::new();
        let mut messages = Vec::new();
        for (kind, visible) in DeviceKind::ALL.into_iter().zip(options.visible) {
            let reading = &self.snapshot.devices[kind.index()];
            let name = self.settings.device_name(kind);
            let message = if let Some(level) = reading.level.filter(|_| reading.online()) {
                let prefix = if reading.estimated { "~" } else { "" };
                format!("{name}: {prefix}{level}% · {}", reading.state_label())
            } else {
                format!("{name}: {}", if reading.message.is_empty() { "Checking…" } else { &reading.message })
            };
            let menu_item = &self.status[kind.index()];
            menu_item.set_text(&message);
            if !visible || !reading.online() { continue; }
            messages.push(message);
            readings.push(WidgetReading {
                id: if kind == DeviceKind::Headset { "usb:g435".into() } else { "usb:madr".into() },
                glyph: if kind == DeviceKind::Headset { DeviceGlyph::Headset } else { DeviceGlyph::Mouse },
                level: reading.level.unwrap_or_default(),
                charging: reading.charging == Some(true),
                full: reading.confirmed_full(),
                low_threshold: self.settings.profile(kind).low_threshold,
            });
        }
        for reading in &self.snapshot.bluetooth {
            readings.push(WidgetReading {
                id: format!("bluetooth:{}", reading.name),
                glyph: DeviceGlyph::for_bluetooth(&reading.name),
                level: reading.percent,
                low_threshold: self.settings.low_threshold,
                ..Default::default()
            });
            messages.push(format!("{}: {}%", reading.name, reading.percent));
        }
        if let Some(widget) = self.widget.as_mut() { widget.update(readings.clone(), options); }
        readings.sort_by_key(|reading| reading.level);
        if let Some(tray) = &self.tray {
            let minimum = readings.first().map(|r| r.level);
            let low = readings.iter().any(|r| !r.charging && r.level <= r.low_threshold);
            let all_full = !readings.is_empty() && readings.iter().all(|r| r.full);
            if let Ok(icon) = BatteryIcon::with_state(minimum, low, all_full) { let _ = tray.set_icon(Some(icon)); }
            let tooltip: String = if messages.is_empty() {
                "Wireless Battery Monitor · waiting for a device".into()
            } else { messages.join("\n").chars().take(120).collect() };
            let _ = tray.set_tooltip(Some(&tooltip));
        }
        if let Some(widget) = self.edge.as_mut() { widget.update(readings, options); }
    }

    fn suspend_devices(&mut self) {
        if self.suspended { return; }
        self.suspended = true;
        self.snapshot.bluetooth.clear();
        if let Some(notifier) = self.notifier.as_mut() { notifier.clear(); }
        self.epoch = self.epoch.wrapping_add(1);
        for alert in &mut self.alerts { alert.disconnected(); }
        for reading in &mut self.snapshot.devices {
            reading.stale = true;
            reading.charging = None;
            reading.message = "System suspended".into();
        }
        let _ = self.snapshot.save();
        let _ = Log::Monitor.event("power", "Suspended; readings invalidated");
        self.update_ui();
    }

    fn resume_devices(&mut self) {
        self.suspended = false;
        self.epoch = self.epoch.wrapping_add(1);
        for alert in &mut self.alerts { alert.disconnected(); }
        for reading in &mut self.snapshot.devices {
            reading.stale = true;
            reading.charging = None;
            reading.message = "Reconnecting after sleep".into();
        }
        self.next_due = [Instant::now(); 2];
        self.last_wall_check = BatteryHistory::now();
        let _ = self.snapshot.save();
        let _ = Log::Monitor.event("power", "Resumed; refreshing devices");
        self.update_ui();
        self.refresh_now();
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
            Event::Widget(WidgetEvent::Refresh | WidgetEvent::DevicesChanged) => self.refresh_now(),
            Event::Widget(WidgetEvent::Exit) => self.exit(event_loop),
            Event::Widget(WidgetEvent::Suspend) => self.suspend_devices(),
            Event::Widget(WidgetEvent::Resume) => { if self.suspended { self.resume_devices(); } },
            Event::Bluetooth(epoch, result) => {
                self.bluetooth_pending = false;
                if epoch == self.epoch && !self.suspended {
                    let interval = self.settings.reconnect_seconds();
                    self.next_bluetooth = Instant::now() + Duration::from_secs(interval);
                    match result {
                        Ok(readings) => {
                            self.bluetooth_failures = 0;
                            if readings != self.snapshot.bluetooth {
                                if self.settings.connection_alerts {
                                    if let Some(notifier) = self.notifier.as_mut() {
                                        for reading in &readings {
                                            if !self.snapshot.bluetooth.iter().any(|previous| previous.name == reading.name) {
                                                notifier.show("Device connected", &format!("{} · {}%", reading.name, reading.percent), true, self.settings.notification_sound);
                                            }
                                        }
                                    }
                                }
                                self.snapshot.bluetooth = readings;
                                self.snapshot.bluetooth_checked_at = BatteryHistory::now();
                                let _ = self.snapshot.save();
                                self.update_ui();
                            } else if !readings.is_empty() && BatteryHistory::now().saturating_sub(self.snapshot.bluetooth_checked_at) >= 16 {
                                self.snapshot.bluetooth_checked_at = BatteryHistory::now();
                                let _ = self.snapshot.save();
                            }
                        }
                        Err(error) => {
                            self.bluetooth_failures = self.bluetooth_failures.saturating_add(1);
                            if self.bluetooth_failures == 1 {
                                let _ = Log::Monitor.event("bluetooth", &error.to_string());
                            }
                            if self.bluetooth_failures >= 2 && !self.snapshot.bluetooth.is_empty() {
                                self.snapshot.bluetooth.clear();
                                let _ = self.snapshot.save();
                                self.update_ui();
                            }
                        }
                    }
                } else {
                    self.next_bluetooth = Instant::now();
                }
            }
            Event::Reading(kind, epoch, result) => {
                if epoch == self.epoch && !self.suspended { self.apply_reading(kind, result); }
                else {
                    self.pending[kind.index()] = false;
                    self.next_due[kind.index()] = Instant::now();
                    self.refresh.set_enabled(!self.pending.iter().any(|v| *v));
                }
                if self.quitting && !self.pending.iter().any(|v| *v) { event_loop.exit(); }
            },
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_settings { self.refresh_settings(now); }
        let mut next = self.next_settings;
        if !self.bluetooth_pending && !self.quitting && !self.suspended {
            if now >= self.next_bluetooth { self.request_bluetooth(); }
            else { next = next.min(self.next_bluetooth); }
        }
        for kind in DeviceKind::ALL {
            let index = kind.index();
            if !self.pending[index] && !self.quitting && !self.suspended {
                if now >= self.next_due[index] { self.request(kind); }
                else { next = next.min(self.next_due[index]); }
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
