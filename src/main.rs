#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod details;

#[cfg(windows)]
fn main() {
    let result = if std::env::args().any(|arg| arg == "--details") {
        details::show()
    } else { app::TrayApp::run() };
    if let Err(error) = result {
        eprintln!("{error}");
        // Release builds have no console; preserve a readable local startup log.
        if let Some(path) = std::env::var_os("LOCALAPPDATA") {
            let dir = std::path::PathBuf::from(path).join("G435Battery");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("startup-error.txt"), error.to_string());
        }
    }
}

#[cfg(not(windows))]
fn main() {
    println!("The tray application runs on Windows. Run cargo test to test the protocol.");
}
