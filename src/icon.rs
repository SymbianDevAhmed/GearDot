//! Small numeric icon, rendered in memory with no image assets.
use tray_icon::Icon;

const DIGITS: [[u8; 5]; 11] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
    [0, 0, 7, 0, 0],
];

pub struct BatteryIcon;

impl BatteryIcon {
    pub fn render(level: Option<u8>) -> Result<Icon, Box<dyn std::error::Error>> {
        let colour = match level {
            None => [95, 105, 120, 255],
            Some(0..=19) => [185, 30, 45, 255],
            Some(20..=49) => [135, 92, 0, 255],
            Some(_) => [15, 115, 70, 255],
        };
        let mut rgba = vec![0; 32 * 32 * 4];
        for y in 1..31 {
            for x in 1..31 {
                if (x == 1 || x == 30) && (y == 1 || y == 30) {
                    continue;
                }
                let i = (y * 32 + x) * 4;
                rgba[i..i + 4].copy_from_slice(&colour);
            }
        }
        let text = level.map(|p| p.to_string()).unwrap_or_else(|| "--".into());
        let scale = if text.len() > 2 { 2 } else { 3 };
        let width = (text.len() * 4 - 1) * scale;
        let x0 = (32 - width) / 2;
        let y0 = (32 - 5 * scale) / 2;
        for (index, ch) in text.chars().enumerate() {
            let glyph = DIGITS[ch.to_digit(10).map(|n| n as usize).unwrap_or(10)];
            for (y, row) in glyph.iter().enumerate() {
                for x in 0..3 {
                    if row & (1 << (2 - x)) == 0 {
                        continue;
                    }
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = x0 + index * 4 * scale + x * scale + dx;
                            let py = y0 + y * scale + dy;
                            let i = (py * 32 + px) * 4;
                            rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                        }
                    }
                }
            }
        }
        Ok(Icon::from_rgba(rgba, 32, 32)?)
    }
}
