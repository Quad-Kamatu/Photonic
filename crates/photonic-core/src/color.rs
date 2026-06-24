use serde::{Deserialize, Serialize};

/// A color in linear sRGB with alpha.
/// All values are in [0.0, 1.0].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    pub const RED: Self = Self {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const GREEN: Self = Self {
        r: 0.0,
        g: 1.0,
        b: 0.0,
        a: 1.0,
    };
    pub const BLUE: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Parse a hex color string: `#RRGGBB` or `#RRGGBBAA`
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
                Some(Self { r, g, b, a: 1.0 })
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()? as f32 / 255.0;
                Some(Self { r, g, b, a })
            }
            _ => None,
        }
    }

    /// Convert to `#RRGGBB` hex string
    pub fn to_hex(&self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}",
            (self.r * 255.0).round() as u8,
            (self.g * 255.0).round() as u8,
            (self.b * 255.0).round() as u8,
        )
    }

    /// Convert to `[r, g, b, a]` as u8
    pub fn to_rgba8(self) -> [u8; 4] {
        [
            (self.r * 255.0).round() as u8,
            (self.g * 255.0).round() as u8,
            (self.b * 255.0).round() as u8,
            (self.a * 255.0).round() as u8,
        ]
    }

    /// Convert to `[r, g, b, a]` as f32 (for GPU uniforms)
    pub fn to_rgba_f32(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn with_alpha(mut self, a: f32) -> Self {
        self.a = a;
        self
    }

    /// Convert to HSL: returns `(hue_degrees, saturation, lightness)`.
    /// Hue ∈ [0, 360), saturation and lightness ∈ [0, 1].
    pub fn to_hsl(self) -> (f32, f32, f32) {
        let r = self.r;
        let g = self.g;
        let b = self.b;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        if (max - min).abs() < 1e-6 {
            return (0.0, 0.0, l); // achromatic
        }
        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if (max - r).abs() < 1e-6 {
            ((g - b) / d).rem_euclid(6.0) * 60.0
        } else if (max - g).abs() < 1e-6 {
            ((b - r) / d + 2.0) * 60.0
        } else {
            ((r - g) / d + 4.0) * 60.0
        };
        (h, s, l)
    }

    /// Create a Color from HSL. Hue ∈ [0, 360), saturation and lightness ∈ [0, 1].
    pub fn from_hsl(h: f32, s: f32, l: f32, a: f32) -> Self {
        if s.abs() < 1e-6 {
            return Self {
                r: l,
                g: l,
                b: l,
                a,
            };
        }
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        let hk = h / 360.0;
        fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
            let t = t.rem_euclid(1.0);
            if t < 1.0 / 6.0 {
                return p + (q - p) * 6.0 * t;
            }
            if t < 1.0 / 2.0 {
                return q;
            }
            if t < 2.0 / 3.0 {
                return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
            }
            p
        }
        Self {
            r: hue_to_rgb(p, q, hk + 1.0 / 3.0),
            g: hue_to_rgb(p, q, hk),
            b: hue_to_rgb(p, q, hk - 1.0 / 3.0),
            a,
        }
    }

    /// Generate a color harmony palette from this color.
    ///
    /// Supported rules: `"complementary"`, `"analogous"`, `"triadic"`,
    /// `"split_complementary"`, `"tetradic"`, `"monochromatic"`.
    /// Returns a Vec of colors (including the base color as the first element).
    pub fn harmony(&self, rule: &str) -> Vec<Color> {
        let (h, s, l) = self.to_hsl();
        let a = self.a;
        let shift = |deg: f32| Color::from_hsl((h + deg).rem_euclid(360.0), s, l, a);
        let mut palette = vec![*self];
        match rule {
            "complementary" => {
                palette.push(shift(180.0));
            }
            "analogous" => {
                palette.insert(0, shift(-30.0));
                palette.push(shift(30.0));
            }
            "triadic" => {
                palette.push(shift(120.0));
                palette.push(shift(240.0));
            }
            "split_complementary" => {
                palette.push(shift(150.0));
                palette.push(shift(210.0));
            }
            "tetradic" => {
                palette.push(shift(90.0));
                palette.push(shift(180.0));
                palette.push(shift(270.0));
            }
            "monochromatic" => {
                for i in 1..=4 {
                    let new_l = (l + i as f32 * 0.15).min(1.0);
                    palette.push(Color::from_hsl(h, s, new_l, a));
                }
            }
            _ => {} // unknown rule — return just base color
        }
        palette
    }

    /// Invert all RGB channels (1.0 − value). Alpha is preserved.
    pub fn invert(self) -> Self {
        Self {
            r: 1.0 - self.r,
            g: 1.0 - self.g,
            b: 1.0 - self.b,
            a: self.a,
        }
    }

    /// Convert to grayscale using the ITU-R BT.601 luminance formula.
    /// Sets R=G=B to perceived brightness; alpha is preserved.
    pub fn to_grayscale(self) -> Self {
        let l = 0.299 * self.r + 0.587 * self.g + 0.114 * self.b;
        Self {
            r: l,
            g: l,
            b: l,
            a: self.a,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}
