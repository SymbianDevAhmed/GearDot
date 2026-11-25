//! Read-only MAD R battery report. No DPI, RGB or firmware settings are changed.
use crate::battery::{BatteryError, BatteryReading};

pub struct MadRProtocol;

impl MadRProtocol {
    pub fn request() -> [u8; 17] {
        let mut report = [0u8; 17];
        report[0] = 8;
        report[1] = 4;
        report[16] = 0x55u8.wrapping_sub(report[..16].iter().fold(0u8, |sum, v| sum.wrapping_add(*v)));
        report
    }

    pub fn parse(report: &[u8]) -> Result<Option<BatteryReading>, BatteryError> {
        if report.len() < 10 || report[..2] != [8, 4] { return Ok(None); }
        let mv = u16::from_be_bytes([report[8], report[9]]);
        if mv == 0 { return Err(BatteryError::NoReply); }
        if report[6] > 100 || report[7] > 1 { return Ok(None); }
        BatteryReading::from_reported(mv, report[6], report[7] == 1).map(Some)
    }
}

#[cfg(windows)]
pub struct MadRReceiver;

#[cfg(windows)]
impl crate::battery::BatterySource for MadRReceiver {
    fn read_battery(&mut self) -> Result<BatteryReading, BatteryError> {
        use hidapi::{HidApi, HidDevice};
        use std::time::{Duration, Instant};

        fn query(device: &HidDevice) -> Result<BatteryReading, BatteryError> {
            let request = MadRProtocol::request();
            let mut buffer = [0u8; 128];
            // Drain a small bounded number of old input frames, not settings.
            for _ in 0..8 {
                if device.read_timeout(&mut buffer, 0).unwrap_or(0) == 0 { break; }
            }
            let written = device.write(&request).map_err(|e| BatteryError::Io(e.to_string()))?;
            if written < request.len() { return Err(BatteryError::Io("short MAD R write".into())); }
            let until = Instant::now() + Duration::from_millis(700);
            while Instant::now() < until {
                let timeout = until.saturating_duration_since(Instant::now()).as_millis().max(1) as i32;
                let n = device.read_timeout(&mut buffer, timeout).map_err(|e| BatteryError::Io(e.to_string()))?;
                if n == 0 { break; }
                if let Some(reading) = MadRProtocol::parse(&buffer[..n])? { return Ok(reading); }
            }
            Err(BatteryError::NoReply)
        }

        let api = HidApi::new().map_err(|e| BatteryError::Io(e.to_string()))?;
        let mut candidates: Vec<_> = api.device_list().filter(|d|
            d.vendor_id() == 0x373B && matches!(d.product_id(), 0x1040 | 0x103F)
                && d.usage_page() == 0xFF02
        ).collect();
        // Prefer cable when connected; both modes represent the same mouse.
        candidates.sort_by_key(|d| if d.product_id() == 0x103F { 0 } else { 1 });
        let mut last_error = BatteryError::ReceiverNotFound;
        for info in candidates {
            match info.open_device(&api) {
                Ok(device) => match query(&device) {
                    Ok(reading) => return Ok(reading),
                    Err(error) => last_error = error,
                },
                Err(error) => last_error = BatteryError::Io(error.to_string()),
            }
        }
        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_bytes_match_documented_report() {
        assert_eq!(MadRProtocol::request(), [8,4,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0x49]);
    }
    #[test]
    fn reads_reported_level_and_charge_flag() {
        let reading = MadRProtocol::parse(&[8,4,0,0,0,0,55,1,0x0f,0xc6]).unwrap().unwrap();
        assert_eq!(reading.percentage(), 55);
        assert_eq!(reading.millivolts(), 4038);
        assert_eq!(reading.charging(), Some(true));
        assert!(!reading.estimated());
    }
    #[test]
    fn rejects_asleep_unrelated_and_invalid_reports() {
        assert_eq!(MadRProtocol::parse(&[8,4,0,0,0,0,55,0,0,0]), Err(BatteryError::NoReply));
        assert_eq!(MadRProtocol::parse(&[8,5,0,0,0,0,55,0,0x0f,0xc6]), Ok(None));
        assert_eq!(MadRProtocol::parse(&[8,4,0,0,0,0,101,0,0x0f,0xc6]), Ok(None));
    }
}
