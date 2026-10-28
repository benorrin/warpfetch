//! The Tokyo Night palette.

pub type Rgb = (u8, u8, u8);

pub const FG: Rgb = (0xc0, 0xca, 0xf5);
pub const COMMENT: Rgb = (0x56, 0x5f, 0x89);
pub const DARK: Rgb = (0x41, 0x48, 0x68);
pub const DEEP_BLUE: Rgb = (0x3d, 0x59, 0xa1);
pub const BLUE: Rgb = (0x7a, 0xa2, 0xf7);
pub const CYAN: Rgb = (0x7d, 0xcf, 0xff);
pub const GREEN: Rgb = (0x9e, 0xce, 0x6a);
pub const PURPLE: Rgb = (0xbb, 0x9a, 0xf7);
pub const RED: Rgb = (0xf7, 0x76, 0x8e);
pub const YELLOW: Rgb = (0xe0, 0xaf, 0x68);
pub const ORANGE: Rgb = (0xff, 0x9e, 0x64);
pub const TEAL: Rgb = (0x73, 0xda, 0xca);

pub const SWATCHES: [Rgb; 8] = [RED, ORANGE, YELLOW, GREEN, TEAL, CYAN, BLUE, PURPLE];

pub fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}
