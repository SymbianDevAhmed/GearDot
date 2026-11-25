//! Windows HID adapter. Only VID 046D / PID 0ADF and vendor usage FF00.
use crate::battery::{BatteryError, BatteryReading, BatterySource};
use crate::protocol::{G435Protocol, Report, REPORT_LEN};
use hidapi::{HidApi, HidDevice};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct G435Receiver;

impl G435Receiver {
    pub fn new() -> Self { Self }

    fn write(device: &HidDevice, report: &Report) -> Result<(), BatteryError> {
        let written = device.write(report).map_err(|e| BatteryError::Io(e.to_string()))?;
        if written != report.len() { return Err(BatteryError::Io("short HID write".into())); }
        Ok(())
    }

    fn exchange(device: &HidDevice) -> Result<BatteryReading, BatteryError> {
        let result = (|| {
            for (report, wait_ms) in G435Protocol::setup() {
                Self::write(device, &report)?;
                std::thread::sleep(Duration::from_millis(wait_ms));
            }
            Self::write(device, &G435Protocol::battery_request())?;
            let deadline = Instant::now() + Duration::from_millis(1200);
            let mut buffer = [0u8; REPORT_LEN];
            while Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let wait_ms = remaining.as_millis().max(1) as i32;
                let count = device.read_timeout(&mut buffer, wait_ms)
                    .map_err(|e| BatteryError::Io(e.to_string()))?;
                if let Some(mv) = G435Protocol::parse_voltage(&buffer[..count])? {
                    return BatteryReading::from_voltage(mv);
                }
                if count == 0 { break; }
            }
            Err(BatteryError::NoReply)
        })();
        // Always release the temporary session, including after failures.
        let cleanup = Self::write(device, &G435Protocol::end_session());
        match (result, cleanup) {
            (Ok(reading), Ok(())) => Ok(reading),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), _) => Err(error),
        }
    }
}

impl BatterySource for G435Receiver {
    fn read_battery(&mut self) -> Result<BatteryReading, BatteryError> {
        let api = HidApi::new().map_err(|e| BatteryError::Io(e.to_string()))?;
        let mut last_error = BatteryError::ReceiverNotFound;
        for info in api.device_list().filter(|i| {
            i.vendor_id() == 0x046d && i.product_id() == 0x0adf && i.usage_page() == 0xff00
        }) {
            match info.open_device(&api) {
                Ok(device) => match Self::exchange(&device) {
                    Ok(reading) => return Ok(reading),
                    Err(error) => last_error = error,
                },
                Err(error) => last_error = BatteryError::Io(error.to_string()),
            }
        }
        Err(last_error)
    }
}
