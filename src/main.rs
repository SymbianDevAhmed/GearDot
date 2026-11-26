#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod bluetooth;
#[cfg(windows)]
mod details;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod runtime;
#[cfg(windows)]
mod taskbar;

#[cfg(windows)]
fn main() {
    use wireless_battery_monitor::diagnostics::Log;

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
    let settings = std::env::args().any(|arg| arg == "--settings");
    let demo = std::env::args().any(|arg| arg == "--demo");
    let window = demo || settings || std::env::args().any(|arg| arg == "--details");
    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        // Separate mutexes permit one monitor and one details/settings window.
        let Some(_guard) = runtime::InstanceGuard::acquire(window)? else {
            return Ok(());
        };
        if window {
            details::show(settings, demo)
        } else {
            app::TrayApp::run()
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
