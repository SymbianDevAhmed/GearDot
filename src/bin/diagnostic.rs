#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use wireless_battery_monitor::{battery::{BatterySource, DeviceKind}, receiver::G435Receiver, mouse::MadRReceiver};
    let mut headset = G435Receiver::new();
    let mut mouse = MadRReceiver;
    let results = [headset.read_battery(), mouse.read_battery()];
    let mut success = false;
    for (kind, result) in DeviceKind::ALL.into_iter().zip(results) {
        match result {
            Ok(reading) => {
                success = true;
                println!("{}: {} mV; {}{}%; charging: {:?}", kind.name(), reading.millivolts(),
                    if reading.estimated() { "~" } else { "" }, reading.percentage(), reading.charging());
            }
            Err(error) => eprintln!("{}: {error}", kind.name()),
        }
    }
    if success { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::FAILURE }
}

#[cfg(not(windows))]
fn main() { println!("This diagnostic requires Windows."); }
