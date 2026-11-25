//! G435 0ADF wire format, derived from the user's successful diagnostic capture.
use crate::battery::BatteryError;

pub const REPORT_LEN: usize = 65;
pub type Report = [u8; REPORT_LEN];

pub struct G435Protocol;

impl G435Protocol {
    fn checksum(bytes: &[u8]) -> u16 {
        bytes.iter().fold(0u16, |mut crc, &byte| {
            crc ^= u16::from(byte) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
            }
            crc
        })
    }

    fn frame(command: u8, body: &[u8]) -> Report {
        assert!(body.len() <= 60);
        let mut report = [0; REPORT_LEN];
        report[1] = command;
        report[2] = body.len() as u8;
        report[5..5 + body.len()].copy_from_slice(body);
        let crc = Self::checksum(&report[1..]).to_le_bytes();
        report[3..5].copy_from_slice(&crc);
        report
    }

    pub fn setup() -> [(Report, u64); 6] {
        [
            (Self::frame(0x84, &[1]), 50),
            (Self::frame(0x85, &[1]), 50),
            (Self::frame(0x80, &[0x50, 0x41, 0x06, 0x01, 0x0d, 0x00, 0x25, 0x34, 0x12,
                0x5a, 0x5a, 0x01, 0, 0, 0, 0xf0, 0, 0, 0]), 450),
            (Self::frame(0x80, &[0x50, 0x41, 0x0e, 2, 1, 0xe0]), 120),
            (Self::frame(0x80, &[0x50, 0x41, 0x0e, 3, 2, 0xe1, 1]), 120),
            (Self::frame(0x80, &[0x50, 0x41, 0x0e, 4, 1, 0xe0]), 120),
        ]
    }

    pub fn battery_request() -> Report {
        Self::frame(0x80, &[0x50, 0x41, 0x06, 5, 1, 0, 0x31])
    }

    pub fn end_session() -> Report { Self::frame(0x85, &[2]) }

    pub fn parse_voltage(report: &[u8]) -> Result<Option<u16>, BatteryError> {
        let payload = match report.len() {
            64 => report,
            65 if report[0] == 0 => &report[1..],
            _ => return Ok(None),
        };
        for window in payload.windows(8) {
            if window[..6] == [0, 7, 0, 6, 0x85, 0] {
                let voltage = u16::from_le_bytes([window[6], window[7]]);
                if !(2500..=4500).contains(&voltage) {
                    return Err(BatteryError::InvalidVoltage);
                }
                return Ok(Some(voltage));
            }
        }
        Ok(None)
    }
}
