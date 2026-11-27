//! BatteryHub Taskbar setup: install, update, reinstall, downgrade or uninstall the taskbar
//! edition, with a window, or silently with /S.
#![windows_subsystem = "windows"]

#[path = "../setup.rs"]
mod setup;
#[path = "../window.rs"]
mod window;

const PAYLOAD: &[u8] = include_bytes!(concat!(env!("BH_PAYLOAD_DIR"), "/batteryhub-taskbar.exe"));

fn main() {
    let ed = batteryhub::install::TASKBAR;
    let o = setup::parse_opts(std::env::args().skip(1));
    let code = if o.silent { setup::run_silent(&ed, PAYLOAD, &o) } else { window::run(ed, PAYLOAD, &o) };
    std::process::exit(code);
}
