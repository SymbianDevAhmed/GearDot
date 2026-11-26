//! Compact battery gauge for the Windows notification area.
use tray_icon::Icon;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceGlyph {
    Headset,
    Mouse,
    Earbuds,
    #[default]
    Bluetooth,
}

impl DeviceGlyph {
    pub fn for_bluetooth(name: &str) -> Self {
        let name = name.to_ascii_lowercase();
        if name.contains("mouse") || name.contains("cd23") {
            Self::Mouse
        } else if ["freearc", "freebuds", "k20i", "life p2 mini", "earbud"]
            .iter().any(|model| name.contains(model)) {
            Self::Earbuds
        } else if name.contains("headset") || name.contains("headphone") {
            Self::Headset
        } else {
            Self::Bluetooth
        }
    }
}

pub struct BatteryIcon;

impl BatteryIcon {
    pub fn render(level: Option<u8>) -> Result<Icon, Box<dyn std::error::Error>> {
        Self::with_state(level, false, false)
    }

    pub fn with_state(level: Option<u8>, low: bool, full: bool) -> Result<Icon, Box<dyn std::error::Error>> {
        let colour = if low { [251, 104, 103, 255] }
            else if full { [75, 210, 154, 255] }
            else { [103, 213, 208, 255] };
        let mut rgba = vec![0; 32 * 32 * 4];
        let mut pixel = |x: usize, y: usize, colour: [u8; 4]| {
            let index = (y * 32 + x) * 4;
            rgba[index..index+4].copy_from_slice(&colour);
        };
        let fill = level.map_or(0, |value| (24 * usize::from(value.min(100))).div_ceil(100));
        for y in 8_usize..24 {
            for x in 1_usize..31 {
                let dx = x.abs_diff(x.clamp(8_usize, 23));
                let dy = y.abs_diff(15_usize).min(y.abs_diff(16_usize));
                let radius = dx * dx + dy * dy;
                if radius > 49 { continue; }
                let shade = if radius > 33 || y == 8 || y == 23 {
                    [133, 145, 153, 255]
                } else if x < 4 + fill {
                    colour
                } else {
                    [49, 60, 66, 255]
                };
                pixel(x, y, shade);
            }
        }
        Ok(Icon::from_rgba(rgba, 32, 32)?)
    }
}
