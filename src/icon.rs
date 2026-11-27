//! Compact battery gauge for the Windows notification area.
use tray_icon::Icon;
use wireless_battery_monitor::{battery::Connection, palette::Rgb, preferences::InputKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceGlyph {
    Headset,
    Mouse,
    Earbuds,
    Keyboard,
    Gamepad,
    #[default]
    Bluetooth,
}

impl DeviceGlyph {
    pub fn label(self) -> &'static str {
        match self {
            Self::Headset => "Headset",
            Self::Mouse => "Mouse",
            Self::Earbuds => "Earbuds",
            Self::Keyboard => "Keyboard",
            Self::Gamepad => "Controller",
            Self::Bluetooth => "Device",
        }
    }

    pub fn tint(self, light: bool) -> Rgb {
        match (self, light) {
            (Self::Mouse, true) => (37, 122, 30),
            (Self::Mouse, false) => (105, 218, 88),
            (Self::Headset | Self::Earbuds, true) => (172, 69, 12),
            (Self::Headset | Self::Earbuds, false) => (255, 140, 64),
            (Self::Gamepad, true) => (30, 87, 198),
            (Self::Gamepad, false) => (93, 149, 255),
            (Self::Keyboard, true) => (0, 108, 151),
            (Self::Keyboard, false) => (62, 194, 244),
            (Self::Bluetooth, true) => (111, 56, 187),
            (Self::Bluetooth, false) => (187, 142, 249),
        }
    }

    pub fn for_input(kind: InputKind) -> Self {
        match kind {
            InputKind::Headset => Self::Headset,
            InputKind::Mouse => Self::Mouse,
            InputKind::Keyboard => Self::Keyboard,
            InputKind::Gamepad => Self::Gamepad,
            InputKind::Other => Self::Bluetooth,
        }
    }

    pub fn for_detected(name: &str, kind: InputKind, connection: Connection) -> Self {
        if connection == Connection::Bluetooth {
            Self::for_bluetooth(name, Some(kind))
        } else {
            Self::for_input(kind)
        }
    }

    pub fn for_bluetooth(name: &str, kind: Option<InputKind>) -> Self {
        let name = name.to_ascii_lowercase();
        if kind == Some(InputKind::Headset)
            || (kind.is_none()
                && [
                    "freearc",
                    "freebuds",
                    "k20i",
                    "life p2 mini",
                    "earbud",
                    "airpods",
                    "galaxy buds",
                ]
                .iter()
                .any(|model| name.contains(model)))
        {
            Self::Earbuds
        } else if let Some(kind) = kind {
            Self::for_input(kind)
        } else if [
            "keyboard", "keychron", "mx keys", "k380", "k480", "k580", "k780",
        ]
        .iter()
        .any(|word| name.contains(word))
        {
            Self::Keyboard
        } else if [
            "controller",
            "gamepad",
            "dualsense",
            "dualshock",
            "8bitdo",
            "joy-con",
        ]
        .iter()
        .any(|word| name.contains(word))
        {
            Self::Gamepad
        } else if name.contains("mouse") || name.contains("cd23") {
            Self::Mouse
        } else if name.contains("headset") || name.contains("headphone") {
            Self::Earbuds
        } else {
            Self::Bluetooth
        }
    }
}

pub struct BatteryIcon;

impl BatteryIcon {
    pub fn render(status: Option<Rgb>) -> Result<Icon, Box<dyn std::error::Error>> {
        let mut rgba = include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon.rgba")).to_vec();
        if let Some((r, g, b)) = status {
            // The detached capsule lives in the upper-right corner of the mark.
            for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                if index % 32 >= 22 && index / 32 < 12 && pixel[3] != 0 {
                    pixel[..3].copy_from_slice(&[r, g, b]);
                }
            }
        }
        Ok(Icon::from_rgba(rgba, 32, 32)?)
    }
}
