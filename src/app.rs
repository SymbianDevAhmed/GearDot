use crate::{
    bluetooth,
    icon::BatteryIcon,
    runtime::Notifier,
    taskbar::{readings_for_display, widget_readings, TaskbarWidget, WidgetEvent, WidgetReading},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::WindowId,
};
use wireless_battery_monitor::{
    battery::{BatteryError, BatteryReading, DeviceKind},
    devices,
    diagnostics::Log,
    palette::{Palette, Rgb},
    preferences::{
        live_data_dir, new_monitor_session, tray_reading, unix_now, Alert, AlertTracker,
        BluetoothReading, InputCharge, LiveReading, LiveSnapshot, Settings, TemporaryMute,
    },
};

const SETTINGS_CHECK: Duration = Duration::from_secs(2);

enum Event {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Widget(WidgetEvent),
    Reading(DeviceKind, u64, Result<BatteryReading, BatteryError>),
    Bluetooth(u64, std::io::Result<Vec<BluetoothReading>>),
    Inputs(
        u64,
        devices::InputScan,
        Option<wireless_battery_monitor::preferences::CachedPhoneBattery>,
    ),
}

/// Presentation and device schedules are independent of battery protocols.
pub struct TrayApp {
    tray: Option<TrayIcon>,
    tray_appearance: Option<Option<Rgb>>,
    tray_tooltip: String,
    widget: Option<TaskbarWidget>,
    edge: Option<TaskbarWidget>,
    bluetooth_failures: u8,
    bluetooth_pending: bool,
    next_bluetooth: Instant,
    next_inputs: Instant,
    inputs_pending: bool,
    missing_driver_rows: BTreeSet<String>,
    missing_bluetooth_rows: BTreeSet<String>,
    notifier: Option<Notifier>,
    refresh: MenuItem,
    details: MenuItem,
    settings_item: MenuItem,
    alerts: [AlertTracker; 2],
    detected_alerts: BTreeMap<String, AlertTracker>,
    snapshot: LiveSnapshot,
    settings: Settings,
    alerts_muted: bool,
    details_process: Option<std::process::Child>,
    quit: MenuItem,
    proxy: EventLoopProxy<Event>,
    pending: [bool; 2],
    confirming_usb_disconnect: [bool; 2],
    quitting: bool,
    suspended: bool,
    epoch: u64,
    last_wall_check: u64,
    next_due: [Instant; 2],
    next_settings: Instant,
    startup_error: Option<String>,
    show_on_start: bool,
}

impl TrayApp {
    pub fn run(show_on_start: bool) -> Result<(), Box<dyn std::error::Error>> {
        let _ = Log::Monitor.event("lifecycle", concat!("Started v", env!("CARGO_PKG_VERSION")));
        let event_loop = EventLoop::<Event>::with_user_event().build()?;
        let proxy = event_loop.create_proxy();
        let menu_proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |event| {
            let _ = menu_proxy.send_event(Event::Menu(event));
        }));
        let tray_proxy = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            let _ = tray_proxy.send_event(Event::Tray(event));
        }));
        let settings = Settings::load();
        let snapshot = LiveSnapshot {
            monitor_session: new_monitor_session(),
            refresh_seconds: settings.refresh_seconds,
            device_intervals: DeviceKind::ALL.map(|k| settings.poll_seconds(k)),
            ..Default::default()
        };
        let _ = snapshot.save();
        let now = Instant::now();
        let mut app = Self {
            tray: None,
            widget: None,
            edge: None,
            bluetooth_failures: 0,
            bluetooth_pending: false,
            next_bluetooth: now,
            next_inputs: now,
            inputs_pending: false,
            missing_driver_rows: BTreeSet::new(),
            missing_bluetooth_rows: BTreeSet::new(),
            notifier: None,
            tray_appearance: None,
            tray_tooltip: String::new(),
            refresh: MenuItem::new("Refresh now", true, None),
            details: MenuItem::new("Battery details", true, None),
            settings_item: MenuItem::new("Settings", true, None),
            alerts: std::array::from_fn(|_| AlertTracker::default()),
            detected_alerts: BTreeMap::new(),
            alerts_muted: settings.battery_alerts_muted
                || TemporaryMute::load().active(unix_now(), &snapshot.monitor_session),
            snapshot,
            settings,
            details_process: None,
            quit: MenuItem::new("Exit", true, None),
            proxy,
            pending: [false; 2],
            confirming_usb_disconnect: [false; 2],
            quitting: false,
            suspended: false,
            epoch: 0,
            last_wall_check: unix_now(),
            next_due: [now; 2],
            next_settings: now,
            startup_error: None,
            show_on_start,
        };
        let event_result = event_loop.run_app(&mut app);
        app.snapshot.monitor_session.clear();
        for reading in &mut app.snapshot.devices {
            reading.stale = true;
            reading.checked_at = 0;
            reading.message = "Monitor exited.".into();
        }
        app.snapshot.bluetooth.clear();
        app.snapshot.input_peripherals.clear();
        let _ = app.snapshot.save();
        let _ = Log::Monitor.event("lifecycle", "Exited");
        event_result?;
        if let Some(error) = app.startup_error {
            return Err(std::io::Error::other(error).into());
        }
        Ok(())
    }

    fn create_tray(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let menu = Menu::new();
        menu.append(&self.details)?;
        menu.append(&self.settings_item)?;
        menu.append(&self.refresh)?;
        menu.append(&self.quit)?;
        self.tray = Some(
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("GearDot · checking for devices")
                .with_menu_on_left_click(false)
                .with_icon(BatteryIcon::render(None)?)
                .build()?,
        );
        let proxy = self.proxy.clone();
        self.widget = TaskbarWidget::new(self.settings.widget_options(), false, move |event| {
            let _ = proxy.send_event(Event::Widget(event));
        });
        let proxy = self.proxy.clone();
        self.edge = TaskbarWidget::new(self.settings.widget_options(), true, move |event| {
            let _ = proxy.send_event(Event::Widget(event));
        });
        self.notifier = Some(Notifier::new());
        self.update_ui();
        Ok(())
    }

    fn open_details(&mut self, settings: bool) {
        if crate::runtime::focus_details() {
            return;
        }
        if self
            .details_process
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
        {
            return;
        }
        if let Ok(exe) = std::env::current_exe() {
            self.details_process = std::process::Command::new(exe)
                .arg(if settings { "--settings" } else { "--details" })
                .spawn()
                .ok();
        }
    }

    fn request(&mut self, kind: DeviceKind) {
        let index = kind.index();
        if self.pending[index] || self.quitting || self.suspended {
            return;
        }
        self.pending[index] = true;
        self.refresh.set_enabled(false);
        let proxy = self.proxy.clone();
        let epoch = self.epoch;
        // Distinct devices do not delay one another while disconnected or asleep.
        std::thread::spawn(move || {
            let reading = devices::read(kind).or_else(|error| {
                if kind == DeviceKind::Headset && error != BatteryError::ReceiverNotFound {
                    return crate::driver_bridge::g435_fallback().ok_or(error);
                }
                Err(error)
            });
            let _ = proxy.send_event(Event::Reading(kind, epoch, reading));
        });
    }

    fn request_bluetooth(&mut self) {
        if self.bluetooth_pending || self.suspended || self.quitting {
            return;
        }
        self.bluetooth_pending = true;
        let proxy = self.proxy.clone();
        let epoch = self.epoch;
        if let Err(error) = std::thread::Builder::new()
            .name("bluetooth-probe".into())
            .spawn(move || {
                let _ = proxy.send_event(Event::Bluetooth(epoch, bluetooth::probe()));
            })
        {
            self.bluetooth_pending = false;
            self.next_bluetooth = Instant::now() + Duration::from_secs(30);
            let _ = Log::Monitor.event("bluetooth", &error.to_string());
        }
    }

    fn request_inputs(&mut self) {
        if self.inputs_pending || self.suspended || self.quitting {
            return;
        }
        self.inputs_pending = true;
        let proxy = self.proxy.clone();
        let epoch = self.epoch;
        if let Err(error) = std::thread::Builder::new()
            .name("input-probe".into())
            .spawn(move || {
                let mut scan = devices::connected_inputs();
                let cached_phone = crate::driver_bridge::poll(&mut scan.devices);
                let _ = proxy.send_event(Event::Inputs(epoch, scan, cached_phone));
            })
        {
            self.inputs_pending = false;
            self.next_inputs = Instant::now() + Duration::from_secs(12);
            let _ = Log::Monitor.event("device driver", &error.to_string());
        }
    }

    fn refresh_now(&mut self) {
        self.next_inputs = Instant::now();
        for kind in DeviceKind::ALL {
            self.request(kind);
        }
        self.request_bluetooth();
        self.request_inputs();
    }

    fn refresh_settings(&mut self, now: Instant) {
        self.next_settings = now + SETTINGS_CHECK;
        let wall = unix_now();
        if wall.saturating_sub(self.last_wall_check) > 20 {
            self.resume_devices();
        }
        self.last_wall_check = wall;
        if let Ok(dir) = live_data_dir() {
            if std::fs::remove_file(dir.join("refresh.request")).is_ok() {
                self.refresh_now();
            }
        }
        let updated = Settings::load();
        let muted = updated.battery_alerts_muted
            || TemporaryMute::load().active(wall, &self.snapshot.monitor_session);
        if muted && !self.alerts_muted {
            if let Some(notifier) = self.notifier.as_mut() {
                notifier.clear();
            }
        }
        self.alerts_muted = muted;
        if updated != self.settings {
            let interval_changed = updated.refresh_seconds != self.settings.refresh_seconds
                || updated.profiles != self.settings.profiles;
            self.settings = updated;
            let _ = Log::Monitor.event("settings", "Preferences applied");
            self.snapshot.refresh_seconds = self.settings.refresh_seconds;
            self.snapshot.device_intervals = DeviceKind::ALL.map(|k| self.settings.poll_seconds(k));
            let _ = self.snapshot.save();
            if interval_changed {
                for (i, due) in self.next_due.iter_mut().enumerate() {
                    if !self.pending[i] {
                        *due = now;
                    }
                }
            }
            self.update_ui();
        }
        if let Some(notifier) = self.notifier.as_mut() {
            notifier.tick();
        }
    }

    fn apply_reading(&mut self, kind: DeviceKind, result: Result<BatteryReading, BatteryError>) {
        let index = kind.index();
        self.pending[index] = false;
        self.refresh.set_enabled(!self.pending.iter().any(|v| *v));
        let now = unix_now();
        if matches!(&result, Err(error) if !matches!(error, BatteryError::ReceiverNotFound))
            && self.snapshot.devices[index].online()
            && !self.confirming_usb_disconnect[index]
        {
            self.confirming_usb_disconnect[index] = true;
            self.next_due[index] =
                Instant::now() + Duration::from_secs(self.settings.reconnect_seconds());
            let _ = Log::Monitor.event(
                "device",
                &format!("{}: battery response missed; checking again", kind.name()),
            );
            return;
        }
        self.confirming_usb_disconnect[index] = false;
        let profile = self.settings.profile(kind);
        let mut delay = Duration::from_secs(self.settings.poll_seconds(kind));
        let status = match result {
            Ok(reading) => {
                if let Some(alert) = self.alerts[index].observe_profile(reading, profile) {
                    let _ = Log::Monitor.event("alert", &format!("{}: {:?}", kind.name(), alert));
                    if let Some(notifier) = self.notifier.as_mut().filter(|_| !self.alerts_muted) {
                        match alert {
                            Alert::Low => notifier.show(
                                "Low battery",
                                &format!(
                                    "{}: {}{}%. Time to charge.",
                                    self.settings.device_name(kind),
                                    if reading.estimated() { "~" } else { "" },
                                    reading.percentage()
                                ),
                                false,
                                self.settings.notification_sound,
                            ),
                            Alert::Full => notifier.show(
                                "Battery full",
                                &format!(
                                    "{} is fully charged (100%).",
                                    self.settings.device_name(kind)
                                ),
                                true,
                                self.settings.notification_sound,
                            ),
                        }
                    }
                }
                // Charge progress is refreshed at least every 15 seconds.
                if reading.charging() == Some(true) && reading.percentage() < 100 {
                    delay = delay.min(Duration::from_secs(self.settings.charge_seconds()));
                }
                LiveReading::from_reading(reading, now)
            }
            Err(error) => {
                delay = Duration::from_secs(self.settings.reconnect_seconds());
                self.alerts[index].disconnected();
                LiveReading {
                    checked_at: now,
                    message: error.to_string(),
                    estimated: kind == DeviceKind::Headset,
                    ..Default::default()
                }
            }
        };
        self.next_due[index] = Instant::now() + delay;
        let previous = &self.snapshot.devices[index];
        if !self.alerts_muted
            && self.settings.connection_alerts
            && previous.checked_at != 0
            && !previous.message.is_empty()
            && !previous.online()
            && status.online()
        {
            if let (Some(level), Some(notifier)) = (status.level, self.notifier.as_mut()) {
                let name = self.settings.device_name(kind);
                let prefix = if status.estimated { "~" } else { "" };
                notifier.show(
                    "Device connected",
                    &format!("{name} · {prefix}{level}%"),
                    true,
                    self.settings.notification_sound,
                );
            }
        }
        if previous.level.is_some() != status.level.is_some()
            || previous.stale != status.stale
            || previous.charging != status.charging
            || previous.message != status.message
        {
            let label = if status.level.is_none() {
                "Unavailable"
            } else {
                status.state_label()
            };
            let _ = Log::Monitor.event("device", &format!("{}: {}", kind.name(), label));
        }
        self.snapshot.devices[index] = status;
        let _ = self.snapshot.save();
        self.update_ui();
    }

    fn update_detected_alerts(&mut self, now: u64) {
        let profile = self.settings.default_profile();
        let mut connected = BTreeSet::new();
        let mut notifications = Vec::new();

        for reading in self.snapshot.connected_bluetooth(now) {
            let id = reading.profile_id();
            let name = self.settings.detected_name(&id, &reading.name);
            let visible = self.settings.detected_visible(&id);
            let tracker = self.detected_alerts.entry(id.to_string()).or_default();
            connected.insert(id);
            if let Some(level) = reading.percent {
                if tracker.observe_telemetry(level, None, false, profile) == Some(Alert::Low)
                    && visible
                {
                    notifications.push(("Low battery", format!("{name} · {level}%")));
                }
            }
        }

        for device in self.snapshot.connected_inputs(now) {
            let id = device.profile_id();
            let name = self.settings.detected_name(&id, &device.name);
            let visible = self.settings.detected_visible(&id);
            let tracker = self.detected_alerts.entry(id.to_string()).or_default();
            connected.insert(id);
            if let Some(battery) = device.battery {
                if battery.stale {
                    continue;
                }
                let charging = battery.charge == InputCharge::Charging;
                let prefix = if battery.estimated { "~" } else { "" };
                let battery_text = format!("{prefix}{}%", battery.percent);
                let alert = tracker.observe_telemetry(
                    battery.percent,
                    Some(charging),
                    battery.estimated,
                    profile,
                );
                match alert.filter(|_| visible) {
                    Some(Alert::Low) => {
                        notifications.push(("Low battery", format!("{name} · {battery_text}")))
                    }
                    Some(Alert::Full) => {
                        notifications.push(("Battery full", format!("{name} · {battery_text}")))
                    }
                    None => {}
                }
            } else if tracker.observe_coarse(device.status, profile) == Some(Alert::Low) && visible
            {
                let level = device.status.level_label().unwrap_or("Low");
                notifications.push(("Low battery", format!("{name} · {level}")));
            }
        }
        self.detected_alerts
            .retain(|id, _| connected.contains(id.as_str()));
        if let Some(notifier) = self.notifier.as_mut().filter(|_| !self.alerts_muted) {
            for (title, message) in notifications {
                notifier.show(title, &message, false, self.settings.notification_sound);
            }
        }
    }

    fn update_ui(&mut self) {
        let options = self.settings.widget_options();
        let readings = widget_readings(&self.settings, &self.snapshot, unix_now());
        if let Some(widget) = self.widget.as_mut() {
            widget.update(
                readings_for_display(&readings, &self.settings, false),
                options,
            );
        }
        if let Some(tray) = &self.tray {
            let target = self.settings.tray_device.as_deref();
            let selected = tray_reading(&readings, target, |reading| {
                (
                    reading.id.as_str(),
                    (!reading.unavailable && !reading.stale).then_some(reading.level),
                )
            });
            let warning = readings.iter().any(|r| r.low());
            let light = crate::taskbar::light_taskbar();
            let palette = Palette::for_theme(light);
            let appearance = if warning {
                Some(palette.low)
            } else {
                selected
                    .map(|r| palette.battery_colour(r.level, r.low_threshold, r.charging, r.full))
            };
            if self.tray_appearance != Some(appearance) {
                if let Ok(icon) = BatteryIcon::render(appearance) {
                    if tray.set_icon(Some(icon)).is_ok() {
                        self.tray_appearance = Some(appearance);
                    }
                }
            }
            let tooltip = match (target, selected) {
                (Some(id), Some(reading)) if reading.id == id => reading.battery_summary(),
                (Some(_), Some(reading)) => {
                    format!("{}\nSelected device unavailable", reading.battery_summary())
                }
                (Some(_), None) => "GearDot · selected device unavailable".into(),
                (None, _) if !readings.is_empty() => readings
                    .iter()
                    .map(WidgetReading::battery_summary)
                    .collect::<Vec<_>>()
                    .join("\n"),
                (None, _) => "GearDot · waiting for a device".into(),
            };
            let tooltip: String = tooltip.chars().take(120).collect();
            if self.tray_tooltip != tooltip && tray.set_tooltip(Some(&tooltip)).is_ok() {
                self.tray_tooltip = tooltip;
            }
        }
        if let Some(widget) = self.edge.as_mut() {
            widget.update(
                readings_for_display(&readings, &self.settings, true),
                options,
            );
        }
    }

    fn suspend_devices(&mut self) {
        if self.suspended {
            return;
        }
        self.suspended = true;
        self.snapshot.bluetooth.clear();
        self.snapshot.input_peripherals.clear();
        if let Some(notifier) = self.notifier.as_mut() {
            notifier.clear();
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.confirming_usb_disconnect = [false; 2];
        for alert in &mut self.alerts {
            alert.disconnected();
        }
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
        self.confirming_usb_disconnect = [false; 2];
        for alert in &mut self.alerts {
            alert.disconnected();
        }
        for reading in &mut self.snapshot.devices {
            reading.stale = true;
            reading.charging = None;
            reading.message = "Reconnecting after sleep".into();
        }
        self.next_due = [Instant::now(); 2];
        self.last_wall_check = unix_now();
        let _ = self.snapshot.save();
        let _ = Log::Monitor.event("power", "Resumed; refreshing devices");
        self.update_ui();
        self.refresh_now();
    }

    fn exit(&mut self, event_loop: &ActiveEventLoop) {
        self.quitting = true;
        if !self.pending.iter().any(|v| *v) {
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<Event> for TrayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.tray.is_none() {
            if let Err(error) = self.create_tray() {
                self.startup_error = Some(error.to_string());
                event_loop.exit();
                return;
            }
            self.refresh_now();
            if std::mem::take(&mut self.show_on_start) {
                self.open_details(false);
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::Menu(event) if event.id() == self.quit.id() => self.exit(event_loop),
            Event::Menu(event) if event.id() == self.refresh.id() => self.refresh_now(),
            Event::Menu(event) if event.id() == self.details.id() => self.open_details(false),
            Event::Menu(event) if event.id() == self.settings_item.id() => self.open_details(true),
            Event::Menu(_) => {}
            Event::Tray(TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }) => self.open_details(false),
            Event::Tray(_) => {}
            Event::Widget(WidgetEvent::Details) => self.open_details(false),
            Event::Widget(WidgetEvent::Settings) => self.open_details(true),
            Event::Widget(WidgetEvent::Refresh | WidgetEvent::DevicesChanged) => self.refresh_now(),
            Event::Widget(WidgetEvent::Exit) => self.exit(event_loop),
            Event::Widget(WidgetEvent::Suspend) => self.suspend_devices(),
            Event::Widget(WidgetEvent::Resume) => {
                if self.suspended {
                    self.resume_devices();
                }
            }
            Event::Inputs(epoch, mut inputs, cached_phone) => {
                self.inputs_pending = false;
                if epoch == self.epoch && !self.suspended {
                    self.next_inputs = Instant::now() + Duration::from_secs(12);
                    let now = unix_now();
                    for device in &mut inputs.devices {
                        let previous = self
                            .snapshot
                            .input_peripherals
                            .iter()
                            .find(|previous| previous.id == device.id)
                            .and_then(|previous| previous.battery);
                        if let Some(reading) = device.battery.as_mut() {
                            reading.checked_at = now;
                            if previous.is_none()
                                && self.settings.connection_alerts
                                && !self.alerts_muted
                                && self.settings.detected_visible(&device.id)
                            {
                                if let Some(notifier) = self.notifier.as_mut() {
                                    let name =
                                        self.settings.detected_name(&device.id, &device.name);
                                    let prefix = if reading.estimated { "~" } else { "" };
                                    notifier.show(
                                        "Device connected",
                                        &format!("{name} · {prefix}{}%", reading.percent),
                                        true,
                                        self.settings.notification_sound,
                                    );
                                }
                            }
                        } else if inputs.hid_error.is_none() {
                            device.battery = previous
                                .filter(|reading| now.saturating_sub(reading.checked_at) <= 60)
                                .map(|mut reading| {
                                    reading.stale = true;
                                    reading
                                });
                        }
                    }
                    let mut missing = BTreeSet::new();
                    for previous in &self.snapshot.input_peripherals {
                        let from_driver = ["batteryhub-", "wdbm-", "headsetcontrol-"]
                            .iter()
                            .any(|prefix| previous.id.starts_with(prefix));
                        if !from_driver
                            || inputs.devices.iter().any(|device| device.id == previous.id)
                        {
                            continue;
                        }
                        missing.insert(previous.id.clone());
                        // A failed probe may miss a connected device for one scan. Confirm
                        // the absence and mark any carried-over percent as a last reading.
                        if !self.missing_driver_rows.contains(&previous.id) {
                            let mut previous = previous.clone();
                            if let Some(battery) = previous.battery.as_mut() {
                                battery.stale = true;
                            }
                            inputs.devices.push(previous);
                        }
                    }
                    self.missing_driver_rows = missing;
                    inputs
                        .devices
                        .sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                    let devices_changed = self.snapshot.input_peripherals != inputs.devices;
                    if devices_changed
                        || self.snapshot.cached_phone != cached_phone
                        || self.snapshot.inputs_hid_error != inputs.hid_error
                        || self.snapshot.audio_error != inputs.audio_error
                        || self.snapshot.inputs_checked_at == 0
                        || now.saturating_sub(self.snapshot.inputs_checked_at) >= 16
                    {
                        self.snapshot.input_peripherals = inputs.devices;
                        self.snapshot.cached_phone = cached_phone;
                        self.snapshot.inputs_hid_error = inputs.hid_error;
                        self.snapshot.audio_error = inputs.audio_error;
                        self.snapshot.inputs_checked_at = now;
                        let _ = self.snapshot.save();
                        self.update_ui();
                    }
                    self.update_detected_alerts(now);
                }
            }
            Event::Bluetooth(epoch, result) => {
                self.bluetooth_pending = false;
                if epoch == self.epoch && !self.suspended {
                    let now = unix_now();
                    let interval = self.settings.reconnect_seconds();
                    self.next_bluetooth = Instant::now() + Duration::from_secs(interval);
                    match result {
                        Ok(mut readings) => {
                            let mut missing = BTreeSet::new();
                            for previous in &self.snapshot.bluetooth {
                                let id = previous.profile_id();
                                if readings.iter().any(|reading| reading.profile_id() == id) {
                                    continue;
                                }
                                if !self.missing_bluetooth_rows.contains(id.as_ref()) {
                                    readings.push(previous.clone());
                                }
                                missing.insert(id.into_owned());
                            }
                            self.missing_bluetooth_rows = missing;
                            readings.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                            self.bluetooth_failures = 0;
                            let first_check = self.snapshot.bluetooth_checked_at == 0;
                            let recovered = self.snapshot.bluetooth_error.take().is_some();
                            if first_check {
                                let _ = Log::Monitor
                                    .event("bluetooth", "Connected-device probe responded");
                            }
                            if readings != self.snapshot.bluetooth {
                                if self.settings.connection_alerts && !self.alerts_muted {
                                    if let Some(notifier) = self.notifier.as_mut() {
                                        for reading in &readings {
                                            let Some(level) = reading.percent else {
                                                continue;
                                            };
                                            if !self.snapshot.bluetooth.iter().any(|previous| {
                                                previous.profile_id() == reading.profile_id()
                                                    && previous.percent.is_some()
                                            }) {
                                                notifier.show(
                                                    "Device connected",
                                                    &format!("{} · {level}%", reading.name),
                                                    true,
                                                    self.settings.notification_sound,
                                                );
                                            }
                                        }
                                    }
                                }
                                self.snapshot.bluetooth = readings;
                                self.snapshot.bluetooth_checked_at = now;
                                let _ = self.snapshot.save();
                                self.update_ui();
                            } else if first_check
                                || recovered
                                || now.saturating_sub(self.snapshot.bluetooth_checked_at) >= 16
                            {
                                self.snapshot.bluetooth_checked_at = now;
                                let _ = self.snapshot.save();
                                self.update_ui();
                            }
                        }
                        Err(error) => {
                            self.bluetooth_failures = self.bluetooth_failures.saturating_add(1);
                            let message = error.to_string();
                            if self.snapshot.bluetooth_error.as_deref() != Some(message.as_str()) {
                                self.snapshot.bluetooth_error = Some(message);
                                let _ = self.snapshot.save();
                            }
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
                    self.update_detected_alerts(now);
                } else {
                    self.next_bluetooth = Instant::now();
                }
            }
            Event::Reading(kind, epoch, result) => {
                if epoch == self.epoch && !self.suspended {
                    self.apply_reading(kind, result);
                } else {
                    self.pending[kind.index()] = false;
                    self.next_due[kind.index()] = Instant::now();
                    self.refresh.set_enabled(!self.pending.iter().any(|v| *v));
                }
                if self.quitting && !self.pending.iter().any(|v| *v) {
                    event_loop.exit();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_settings {
            self.refresh_settings(now);
        }
        let mut next = self.next_settings;
        if !self.bluetooth_pending && !self.quitting && !self.suspended {
            if now >= self.next_bluetooth {
                self.request_bluetooth();
            } else {
                next = next.min(self.next_bluetooth);
            }
        }
        if !self.inputs_pending && !self.quitting && !self.suspended {
            if now >= self.next_inputs {
                self.request_inputs();
            } else {
                next = next.min(self.next_inputs);
            }
        }
        for kind in DeviceKind::ALL {
            let index = kind.index();
            if !self.pending[index] && !self.quitting && !self.suspended {
                if now >= self.next_due[index] {
                    self.request(kind);
                } else {
                    next = next.min(self.next_due[index]);
                }
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
