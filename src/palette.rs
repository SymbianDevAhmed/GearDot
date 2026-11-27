//! Shared colours for the app, the screen-edge widget, and the tray.

use serde::{Deserialize, Serialize};

pub type Rgb = (u8, u8, u8);

pub struct Palette {
    pub background: Rgb,
    pub surface: Rgb,
    pub border: Rgb,
    pub selection: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub charging: Rgb,
    pub full: Rgb,
    pub low: Rgb,
}

impl Palette {
    pub fn for_theme(light: bool) -> &'static Self {
        const DARK: Palette = Palette {
            background: (16, 21, 38),
            surface: (27, 35, 64),
            border: (53, 62, 97),
            selection: (48, 59, 106),
            text: (237, 240, 255),
            muted: (170, 182, 215),
            accent: (169, 179, 255),
            charging: (83, 222, 243),
            full: (101, 232, 170),
            low: (255, 134, 171),
        };
        const LIGHT: Palette = Palette {
            background: (238, 241, 253),
            surface: (255, 255, 255),
            border: (207, 214, 241),
            selection: (220, 226, 255),
            text: (20, 32, 67),
            muted: (75, 88, 122),
            accent: (69, 73, 168),
            charging: (0, 105, 125),
            full: (8, 116, 73),
            low: (160, 40, 88),
        };
        if light {
            &LIGHT
        } else {
            &DARK
        }
    }

    pub fn battery_level(&self, level: u8, low_threshold: u8) -> Rgb {
        let light = self.surface.0 >= 128;
        match (level <= low_threshold, level >= 70, light) {
            (true, _, false) => (249, 117, 130),
            (true, _, true) => (160, 33, 54),
            (false, true, false) => (113, 225, 153),
            (false, true, true) => (20, 112, 61),
            (false, false, false) => (255, 199, 92),
            (false, false, true) => (139, 86, 0),
        }
    }

    pub fn battery_colour(&self, level: u8, low_threshold: u8, charging: bool, full: bool) -> Rgb {
        if full {
            self.battery_level(100, low_threshold)
        } else if charging {
            self.charging
        } else {
            self.battery_level(level, low_threshold)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisualTheme {
    Ion,
    Carbon,
    Porcelain,
    Orchid,
    Moss,
    Ember,
    #[default]
    Obsidian,
    Arctic,
    Dune,
    Bloom,
    Lagoon,
    Matcha,
    Glass,
}

impl VisualTheme {
    pub const ALL: [Self; 13] = [
        Self::Ion,
        Self::Carbon,
        Self::Porcelain,
        Self::Orchid,
        Self::Moss,
        Self::Ember,
        Self::Obsidian,
        Self::Arctic,
        Self::Dune,
        Self::Bloom,
        Self::Lagoon,
        Self::Matcha,
        Self::Glass,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ion => "Ion",
            Self::Carbon => "Carbon",
            Self::Porcelain => "Porcelain",
            Self::Orchid => "Orchid",
            Self::Moss => "Moss",
            Self::Ember => "Ember",
            Self::Obsidian => "Obsidian",
            Self::Arctic => "Arctic",
            Self::Dune => "Dune",
            Self::Bloom => "Bloom",
            Self::Lagoon => "Lagoon",
            Self::Matcha => "Matcha",
            Self::Glass => "Liquid Glass",
        }
    }

    pub fn is_light(self, system_light: bool) -> bool {
        match self {
            Self::Ion | Self::Glass => system_light,
            Self::Porcelain | Self::Arctic | Self::Bloom | Self::Matcha => true,
            _ => false,
        }
    }

    pub fn palette(self, light: bool) -> &'static Palette {
        match self {
            Self::Ion | Self::Glass => Palette::for_theme(light),
            Self::Carbon => &CARBON,
            Self::Porcelain => &PORCELAIN,
            Self::Orchid => &ORCHID,
            Self::Moss => &MOSS,
            Self::Ember => &EMBER,
            Self::Obsidian => &OBSIDIAN,
            Self::Arctic => &ARCTIC,
            Self::Dune => &DUNE,
            Self::Bloom => &BLOOM,
            Self::Lagoon => &LAGOON,
            Self::Matcha => &MATCHA,
        }
    }
}

const CARBON: Palette = Palette {
    background: (17, 23, 27),
    surface: (28, 38, 44),
    border: (57, 76, 86),
    selection: (37, 57, 67),
    text: (238, 247, 250),
    muted: (177, 196, 204),
    accent: (154, 222, 255),
    charging: (113, 246, 226),
    full: (170, 234, 167),
    low: (255, 158, 146),
};

const PORCELAIN: Palette = Palette {
    background: (244, 239, 230),
    surface: (255, 252, 246),
    border: (215, 208, 193),
    selection: (226, 229, 250),
    text: (34, 38, 59),
    muted: (94, 96, 112),
    accent: (53, 70, 168),
    charging: (0, 109, 120),
    full: (38, 116, 79),
    low: (166, 44, 82),
};

const ORCHID: Palette = Palette {
    background: (26, 19, 35),
    surface: (42, 29, 54),
    border: (73, 52, 89),
    selection: (66, 44, 86),
    text: (249, 240, 255),
    muted: (206, 186, 218),
    accent: (223, 170, 255),
    charging: (138, 223, 255),
    full: (156, 232, 189),
    low: (255, 162, 159),
};

const MOSS: Palette = Palette {
    background: (19, 28, 24),
    surface: (32, 46, 39),
    border: (63, 84, 73),
    selection: (52, 70, 58),
    text: (242, 248, 238),
    muted: (180, 201, 183),
    accent: (209, 238, 164),
    charging: (141, 219, 255),
    full: (134, 230, 192),
    low: (255, 152, 142),
};

const EMBER: Palette = Palette {
    background: (37, 23, 25),
    surface: (55, 35, 38),
    border: (88, 60, 64),
    selection: (81, 50, 56),
    text: (255, 240, 236),
    muted: (217, 186, 184),
    accent: (255, 180, 157),
    charging: (140, 220, 235),
    full: (160, 229, 176),
    low: (255, 148, 178),
};

const OBSIDIAN: Palette = Palette {
    background: (8, 9, 11),
    surface: (22, 24, 29),
    border: (53, 56, 63),
    selection: (48, 51, 59),
    text: (250, 249, 245),
    muted: (197, 196, 196),
    accent: (231, 224, 208),
    charging: (155, 215, 249),
    full: (167, 225, 195),
    low: (255, 165, 144),
};

const ARCTIC: Palette = Palette {
    background: (229, 242, 248),
    surface: (248, 252, 254),
    border: (189, 212, 223),
    selection: (203, 228, 238),
    text: (21, 48, 74),
    muted: (67, 97, 121),
    accent: (8, 95, 137),
    charging: (19, 95, 103),
    full: (38, 111, 83),
    low: (169, 45, 80),
};

const DUNE: Palette = Palette {
    background: (33, 27, 17),
    surface: (48, 39, 26),
    border: (83, 67, 44),
    selection: (75, 57, 39),
    text: (249, 236, 217),
    muted: (216, 194, 162),
    accent: (239, 192, 128),
    charging: (161, 223, 252),
    full: (185, 222, 154),
    low: (255, 159, 155),
};

const BLOOM: Palette = Palette {
    background: (245, 234, 241),
    surface: (255, 248, 252),
    border: (224, 197, 213),
    selection: (239, 219, 234),
    text: (60, 33, 53),
    muted: (109, 77, 98),
    accent: (139, 52, 106),
    charging: (0, 108, 120),
    full: (53, 108, 71),
    low: (158, 55, 56),
};

const LAGOON: Palette = Palette {
    background: (7, 29, 34),
    surface: (16, 44, 51),
    border: (44, 78, 87),
    selection: (33, 66, 74),
    text: (228, 249, 246),
    muted: (166, 199, 197),
    accent: (103, 232, 188),
    charging: (149, 207, 255),
    full: (217, 238, 153),
    low: (255, 153, 157),
};

const MATCHA: Palette = Palette {
    background: (233, 239, 225),
    surface: (247, 250, 241),
    border: (201, 214, 191),
    selection: (216, 229, 202),
    text: (34, 52, 38),
    muted: (80, 103, 81),
    accent: (53, 94, 55),
    charging: (9, 103, 122),
    full: (61, 103, 40),
    low: (156, 48, 84),
};

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(rgb: Rgb) -> f64 {
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgb.0) + 0.7152 * linear(rgb.1) + 0.0722 * linear(rgb.2)
    }

    #[test]
    fn theme_palettes_keep_card_text_and_statuses_readable() {
        for theme in VisualTheme::ALL {
            for system_light in [false, true] {
                let palette = theme.palette(theme.is_light(system_light));
                let surface = luminance(palette.surface);
                for rgb in [
                    palette.text,
                    palette.muted,
                    palette.accent,
                    palette.charging,
                    palette.full,
                    palette.low,
                ] {
                    let foreground = luminance(rgb);
                    assert!(
                        (surface.max(foreground) + 0.05) / (surface.min(foreground) + 0.05) >= 4.5,
                        "Low contrast in {}",
                        theme.label()
                    );
                }
                let serialized = serde_json::to_string(&theme).unwrap();
                assert_eq!(
                    serde_json::from_str::<VisualTheme>(&serialized).unwrap(),
                    theme
                );
            }
        }
    }

    #[test]
    fn battery_bands_keep_distinct_readable_colours() {
        for theme in VisualTheme::ALL {
            for mode in [false, true] {
                let palette = theme.palette(theme.is_light(mode));
                let surface = luminance(palette.surface);
                for threshold in 5..=50 {
                    let low = palette.battery_level(threshold, threshold);
                    let medium = palette.battery_level(threshold + 1, threshold);
                    let high = palette.battery_level(70, threshold);
                    assert_ne!(low, medium);
                    assert_ne!(medium, high);
                    assert_ne!(high, low);
                    assert_eq!(palette.battery_colour(10, threshold, true, true), high);
                    assert_eq!(
                        palette.battery_colour(10, threshold, true, false),
                        palette.charging
                    );
                    for percent in 0..=100 {
                        let rgb = palette.battery_level(percent, threshold);
                        assert_eq!(
                            rgb,
                            if percent <= threshold {
                                low
                            } else if percent < 70 {
                                medium
                            } else {
                                high
                            }
                        );
                        let foreground = luminance(rgb);
                        assert!(
                            (surface.max(foreground) + 0.05) / (surface.min(foreground) + 0.05)
                                >= 4.5,
                            "Low battery colour contrast in {} at {percent}%: {rgb:?}",
                            theme.label()
                        );
                    }
                }
            }
        }
    }
}
