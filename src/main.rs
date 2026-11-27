#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod background;
#[cfg(windows)]
mod bluetooth;
#[cfg(windows)]
mod details;
#[cfg(any(windows, test))]
mod driver_bridge;
#[cfg(windows)]
mod edge_hover;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod runtime;
#[cfg(windows)]
mod taskbar;
#[cfg(any(windows, test))]
mod taskbar_anchor;
#[cfg(any(windows, test))]
mod window_motion;

#[cfg(windows)]
fn main() {
    use wireless_battery_monitor::diagnostics::Log;

    if std::env::args().any(|arg| arg == "--clear-installed-startup") {
        if matches!(runtime::startup_enabled(), Ok(true)) {
            let _ = runtime::set_startup(false);
        }
        return;
    }

    let g435_probe = std::env::args().any(|arg| arg == "--batteryhub-g435-probe");
    if g435_probe || std::env::args().any(|arg| arg == "--batteryhub-probe") {
        if driver_bridge::write_batteryhub_response(g435_probe).is_err() {
            std::process::exit(1);
        }
        return;
    }
    let probe = std::env::args().any(|arg| arg == "--bluetooth-probe");
    std::panic::set_hook(Box::new(move |panic| {
        let _ = Log::Monitor.event(
            if probe { "bluetooth-panic" } else { "panic" },
            &panic.to_string(),
        );
    }));
    if probe {
        if let Err(error) = bluetooth::write_probe_response() {
            let _ = Log::Monitor.event("bluetooth-probe", &error.to_string());
            std::process::exit(1);
        }
        return;
    }
    let background = std::env::args().any(|arg| arg == "--background");
    let settings = std::env::args().any(|arg| arg == "--settings");
    let demo = std::env::args().any(|arg| arg == "--demo");
    let window = demo || settings || std::env::args().any(|arg| arg == "--details");
    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        let Some(_guard) = runtime::InstanceGuard::acquire(window)? else {
            if !background && !runtime::focus_details() && !window {
                std::process::Command::new(std::env::current_exe()?)
                    .arg("--details")
                    .spawn()?;
            }
            return Ok(());
        };
        if window {
            details::show(settings, demo)
        } else {
            app::TrayApp::run(!background)
        }
    })();
    if let Err(error) = result {
        let _ = Log::Monitor.event("runtime-error", &error.to_string());
        eprintln!("{error}");
        // Release builds have no console; preserve a readable local startup log.
        if let Ok(dir) = wireless_battery_monitor::preferences::data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("startup-error.txt"), error.to_string());
        }
    }
}

#[cfg(not(windows))]
fn main() {
    println!("The tray application runs on Windows. Run cargo test to test the protocol.");
}
