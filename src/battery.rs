//! Domain model: no Windows, USB or user-interface dependencies.
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryReading {
    millivolts: u16,
    percentage: u8,
    estimated: bool,
    charging: Option<bool>,
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
        Ok(Self { millivolts, percentage, estimated: true, charging: None })
    }

    pub fn from_reported(millivolts: u16, percentage: u8, charging: bool) -> Result<Self, BatteryError> {
        if !(2500..=4500).contains(&millivolts) || percentage > 100 {
            return Err(BatteryError::InvalidVoltage);
        }
        Ok(Self { millivolts, percentage, estimated: false, charging: Some(charging) })
    }

    pub fn estimated(self) -> bool { self.estimated }
    pub fn charging(self) -> Option<bool> { self.charging }
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
            Self::ReceiverNotFound => write!(f, "USB receiver not found"),
            Self::NoReply => write!(f, "No battery reply; device may be asleep or off"),
            Self::InvalidVoltage => write!(f, "Invalid battery voltage"),
            Self::Io(message) => write!(f, "HID error: {message}"),
        }
    }
}

impl std::error::Error for BatteryError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceKind {
    #[default]
    Headset,
    Mouse,
}
impl DeviceKind {
    pub const ALL: [Self; 2] = [Self::Headset, Self::Mouse];
    pub fn name(self) -> &'static str {
        match self { Self::Headset => "Logitech G435", Self::Mouse => "VXE MAD R" }
    }
    pub fn index(self) -> usize {
        match self { Self::Headset => 0, Self::Mouse => 1 }
    }
}

/// Abstraction: the UI does not depend on a particular headset or USB API.
pub trait BatterySource {
    fn read_battery(&mut self) -> Result<BatteryReading, BatteryError>;
}
