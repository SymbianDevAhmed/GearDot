//! Compact battery gauge for the Windows notification area.
use tray_icon::Icon;

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
        let frame = [166, 177, 188, 255];
        let fill = level.map_or(0, |value| (19 * usize::from(value.min(100))).div_ceil(100));
        for y in 7..25 {
            for x in 2..28 {
                let corner = (x == 2 || x == 27) && (y == 7 || y == 24);
                if corner { continue; }
                if !(4..26).contains(&x) || !(9..23).contains(&y) { pixel(x, y, frame); }
                else if (5..5+fill).contains(&x) && (10..22).contains(&y) { pixel(x, y, colour); }
            }
        }
        for y in 12..20 { for x in 28..31 { pixel(x, y, frame); } }
        Ok(Icon::from_rgba(rgba, 32, 32)?)
    }
}
