#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use g435_battery::{battery::BatterySource, receiver::G435Receiver};
    match G435Receiver::new().read_battery() {
        Ok(reading) => {
            println!("G435: {} mV; estimated {}%", reading.millivolts(), reading.percentage());
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() { println!("This diagnostic requires Windows and a 0ADF receiver."); }
