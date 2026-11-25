//! Domain model: no Windows, USB or user-interface dependencies.
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryReading {
    millivolts: u16,
    percentage: u8,
}

impl BatteryReading {
    pub fn from_voltage(millivolts: u16) -> Result<Self, BatteryError> {
        if !(2500..=4500).contains(&millivolts) {
            return Err(BatteryError::InvalidVoltage);
        }
        // Piecewise approximation; never represent this as an exact fuel gauge.
        const CURVE: [(u16, u8); 16] = [
            (3400, 0), (3500, 5), (3550, 10), (3600, 15),
            (3650, 25), (3700, 35), (3750, 45), (3800, 55),
            (3850, 62), (3900, 70), (3950, 75), (4000, 80),
            (4050, 85), (4100, 90), (4150, 95), (4200, 100),
        ];
        let percentage = if millivolts <= CURVE[0].0 {
            0
        } else if millivolts >= CURVE[15].0 {
            100
        } else {
            CURVE.windows(2).find_map(|pair| {
                let (v0, p0) = pair[0];
                let (v1, p1) = pair[1];
                if millivolts < v0 || millivolts > v1 {
                    return None;
                }
                let width = u32::from(v1 - v0);
                let numerator = u32::from(millivolts - v0) * u32::from(p1 - p0);
                Some(p0 + ((numerator + width / 2) / width) as u8)
            }).ok_or(BatteryError::InvalidVoltage)?
        };
        Ok(Self { millivolts, percentage })
    }

    pub fn millivolts(self) -> u16 { self.millivolts }
    pub fn percentage(self) -> u8 { self.percentage }
    pub fn is_low(self) -> bool { self.percentage < 20 }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatteryError {
    ReceiverNotFound,
    NoReply,
    InvalidVoltage,
    Io(String),
}

impl fmt::Display for BatteryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReceiverNotFound => write!(f, "G435 receiver (046D:0ADF) not found"),
            Self::NoReply => write!(f, "No battery reply; check that the headset is on"),
            Self::InvalidVoltage => write!(f, "Invalid battery voltage"),
            Self::Io(message) => write!(f, "HID error: {message}"),
        }
    }
}

impl std::error::Error for BatteryError {}

/// Abstraction: the UI does not depend on a particular headset or USB API.
pub trait BatterySource {
    fn read_battery(&mut self) -> Result<BatteryReading, BatteryError>;
}
