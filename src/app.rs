use crate::icon::BatteryIcon;
use g435_battery::battery::{BatteryError, BatteryReading, BatterySource};
use g435_battery::receiver::G435Receiver;
use g435_battery::history::BatteryHistory;
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
    Reading(Result<BatteryReading, BatteryError>),
}

/// Presentation layer owns all tray resources; HID requests run on background threads.
pub struct TrayApp {
    tray: Option<TrayIcon>,
    status: MenuItem,
    refresh: MenuItem,
    details: MenuItem,
    history: BatteryHistory,
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
            status: MenuItem::new("Checking G435...", false, None),
            refresh: MenuItem::new("Refresh now", true, None),
            details: MenuItem::new("Battery details", true, None),
            history: BatteryHistory::load(),
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
        menu.append(&self.details)?;
        menu.append(&self.refresh)?;
        menu.append(&self.quit)?;
        self.tray = Some(TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Logitech G435: checking...")
            .with_menu_on_left_click(false)
            .with_icon(BatteryIcon::render(None)?)
            .build()?);
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
            let mut source = G435Receiver::new();
            let _ = proxy.send_event(Event::Reading(source.read_battery()));
        });
    }

    fn apply_reading(&mut self, result: Result<BatteryReading, BatteryError>) {
        self.pending = false;
        self.refresh.set_enabled(true);
        let (level, message) = match result {
            Ok(reading) => {
                let _ = self.history.record(reading);
                (Some(reading.percentage()),
                format!("G435: ~{}% ({} mV){}", reading.percentage(), reading.millivolts(),
                    if reading.is_low() { " - low battery" } else { "" }))
            },
            Err(error) => (None, format!("G435: {error}")),
        };
        self.status.set_text(&message);
        if let Some(tray) = &self.tray {
            if let Ok(icon) = BatteryIcon::render(level) {
                let _ = tray.set_icon(Some(icon));
            }
            let _ = tray.set_tooltip(Some(&message));
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
