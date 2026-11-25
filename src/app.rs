use crate::icon::BatteryIcon;
use crate::taskbar::{TaskbarWidget, WidgetEvent, WidgetReading};
use wireless_battery_monitor::battery::{BatteryError, BatteryReading, BatterySource, DeviceKind};
use wireless_battery_monitor::receiver::G435Receiver;
use wireless_battery_monitor::mouse::MadRReceiver;
use wireless_battery_monitor::history::BatteryHistory;
use std::time::{Duration, Instant};
use tray_icon::{menu::{Menu, MenuEvent, MenuItem}, TrayIcon, TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::WindowId,
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

enum Event {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Widget(WidgetEvent),
    Reading([Result<BatteryReading, BatteryError>; 2]),
}

/// Presentation layer owns all tray resources; HID requests run on background threads.
pub struct TrayApp {
    tray: Option<TrayIcon>,
    widget: Option<TaskbarWidget>,
    status: MenuItem,
    mouse_status: MenuItem,
    refresh: MenuItem,
    details: MenuItem,
    histories: [BatteryHistory; 2],
    details_process: Option<std::process::Child>,
    quit: MenuItem,
    proxy: EventLoopProxy<Event>,
    pending: bool,
    quitting: bool,
    next_refresh: Instant,
    startup_error: Option<String>,
}

impl TrayApp {
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
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
        let mut app = Self {
            tray: None,
            widget: None,
            status: MenuItem::new("Checking G435...", false, None),
            mouse_status: MenuItem::new("Checking MAD R...", false, None),
            refresh: MenuItem::new("Refresh now", true, None),
            details: MenuItem::new("Battery details", true, None),
            histories: DeviceKind::ALL.map(BatteryHistory::load),
            details_process: None,
            quit: MenuItem::new("Exit", true, None),
            proxy,
            pending: false,
            quitting: false,
            next_refresh: Instant::now(),
            startup_error: None,
        };
        event_loop.run_app(&mut app)?;
        if let Some(error) = app.startup_error {
            return Err(std::io::Error::other(error).into());
        }
        Ok(())
    }

    fn create_tray(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let menu = Menu::new();
        menu.append(&self.status)?;
        menu.append(&self.mouse_status)?;
        menu.append(&self.details)?;
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
        Ok(())
    }

    fn open_details(&mut self) {
        if let Some(child) = self.details_process.as_mut() {
            if matches!(child.try_wait(), Ok(None)) { return; }
        }
        if let Ok(exe) = std::env::current_exe() {
            self.details_process = std::process::Command::new(exe).arg("--details").spawn().ok();
        }
    }

    fn request_reading(&mut self) {
        if self.pending { return; }
        self.pending = true;
        self.refresh.set_enabled(false);
        self.next_refresh = Instant::now() + REFRESH_INTERVAL;
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            // Same BatterySource abstraction, two independent hardware adapters.
            let mut headset = G435Receiver::new();
            let mut mouse = MadRReceiver;
            let _ = proxy.send_event(Event::Reading([headset.read_battery(), mouse.read_battery()]));
        });
    }

    fn apply_reading(&mut self, results: [Result<BatteryReading, BatteryError>; 2]) {
        self.pending = false;
        self.refresh.set_enabled(true);
        let mut readings = [WidgetReading::default(); 2];
        let mut messages = Vec::new();
        for (kind, result) in DeviceKind::ALL.into_iter().zip(results) {
            let index = kind.index();
            let history = &mut self.histories[index];
            let message = match result {
                Ok(reading) => {
                    let _ = history.record(reading);
                    readings[index] = WidgetReading { level: Some(reading.percentage()), charging: reading.charging() == Some(true), stale: false };
                    format!("{}: {}{}% ({} mV){}", kind.name(), if reading.estimated() { "~" } else { "" },
                        reading.percentage(), reading.millivolts(), if reading.charging() == Some(true) { " • charging" } else { "" })
                },
                Err(error) => {
                    // A sleeping mouse may not answer. Preserve its last sample briefly,
                    // but never present an unplugged receiver or old sample as current.
                    if kind == DeviceKind::Mouse && !matches!(error, BatteryError::ReceiverNotFound) {
                        if let Some(previous) = history.latest().filter(|s| BatteryHistory::now().saturating_sub(s.at) <= 1800) {
                            readings[index] = WidgetReading { level: Some(previous.percent), charging: false, stale: true };
                        }
                    }
                    if let Some(level) = readings[index].level {
                        format!("{}: ~{}% (last reading; {})", kind.name(), level, error)
                    } else {
                        format!("{}: {}", kind.name(), error)
                    }
                },
            };
            messages.push(message);
        }
        self.status.set_text(&messages[0]);
        self.mouse_status.set_text(&messages[1]);
        if let Some(widget) = self.widget.as_mut() { widget.set_readings(readings); }
        if let Some(tray) = &self.tray {
            if let Ok(icon) = BatteryIcon::render(readings[0].level.or(readings[1].level)) {
                let _ = tray.set_icon(Some(icon));
            }
            let tooltip = messages.join("\n");
            let tooltip: String = tooltip.chars().take(120).collect();
            let _ = tray.set_tooltip(Some(&tooltip));
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
            self.request_reading();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::Menu(event) if event.id() == self.quit.id() => {
                if self.pending { self.quitting = true; } else { event_loop.exit(); }
            },
            Event::Menu(event) if event.id() == self.refresh.id() => self.request_reading(),
            Event::Menu(event) if event.id() == self.details.id() => self.open_details(),
            Event::Menu(_) => {},
            Event::Tray(TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) => self.open_details(),
            Event::Tray(_) => {},
            Event::Widget(WidgetEvent::Details) => self.open_details(),
            Event::Widget(WidgetEvent::Refresh) => self.request_reading(),
            Event::Widget(WidgetEvent::Exit) => {
                if self.pending { self.quitting = true; } else { event_loop.exit(); }
            },
            Event::Widget(WidgetEvent::Visibility(visible)) => {
                if let Some(tray) = &self.tray { let _ = tray.set_visible(!visible); }
            },
            Event::Reading(reading) => {
                self.apply_reading(reading);
                if self.quitting { event_loop.exit(); }
            },
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if Instant::now() >= self.next_refresh && !self.pending {
            self.request_reading();
        }
        event_loop.set_control_flow(if self.pending {
            ControlFlow::Wait
        } else {
            ControlFlow::WaitUntil(self.next_refresh)
        });
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
