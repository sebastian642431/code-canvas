//! The window / taskbar icon. The executable's own icon (shown by the file
//! manager and shortcuts) is embedded by `build.rs` on Windows.
//!
//! Regenerate the assets with `python scripts/make_icon.py`.

use winit::window::Icon;

const SIZE: u32 = 64;
static RGBA: &[u8] = include_bytes!("../assets/icon_64.rgba");

pub fn window_icon() -> Option<Icon> {
    Icon::from_rgba(RGBA.to_vec(), SIZE, SIZE).ok()
}

/// The larger icon Windows shows in the taskbar and in Alt+Tab.
#[cfg(windows)]
pub fn taskbar_icon() -> Option<Icon> {
    Icon::from_rgba(include_bytes!("../assets/icon_128.rgba").to_vec(), 128, 128).ok()
}
