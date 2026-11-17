#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub origin: [i32; 2],
    pub size: [u32; 2],
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub size: [f32; 2],
    pub position: [i32; 2],
}

const TASKBAR: f32 = 56.0;
const MARGIN: f32 = 24.0;
const SMALLEST: [f32; 2] = [720.0, 420.0];

fn room(screen: &Screen) -> [f32; 2] {
    let logical = [screen.size[0] as f32 / screen.scale, screen.size[1] as f32 / screen.scale];
    [(logical[0] - MARGIN).max(SMALLEST[0]), (logical[1] - TASKBAR - MARGIN).max(SMALLEST[1])]
}

fn holds(screen: &Screen, position: [i32; 2], width: f32) -> bool {
    let (x, y) = (position[0], position[1]);
    let (left, top) = (screen.origin[0], screen.origin[1]);
    let (right, bottom) = (left + screen.size[0] as i32, top + screen.size[1] as i32);
    let grab = (120.0 * screen.scale) as i32;
    let wide = (width * screen.scale) as i32;
    x + wide > left + grab && x < right - grab && y >= top - 8 && y < bottom - grab
}

pub fn place(screens: &[Screen], first: Option<[i32; 2]>, size: Option<[f32; 2]>, position: Option<[i32; 2]>) -> Place {
    let fallback = Screen { origin: [0, 0], size: [1920, 1080], scale: 1.0 };
    let wanted = size.filter(|s| s[0] >= 400.0 && s[1] >= 300.0);
    let kept = position.and_then(|p| {
        let width = wanted.map_or(SMALLEST[0], |s| s[0]);
        screens.iter().find(|s| holds(s, p, width)).map(|s| (*s, p))
    });
    let screen = match kept {
        Some((screen, _)) => screen,
        None => first
            .and_then(|origin| screens.iter().find(|s| s.origin == origin))
            .or(screens.first())
            .copied()
            .unwrap_or(fallback),
    };
    let room = room(&screen);
    let size = match wanted {
        Some(s) => [s[0].min(room[0]), s[1].min(room[1])],
        None => [(room[0] * 0.9).round(), (room[1] * 0.92).round()],
    };
    let pixels = [(size[0] * screen.scale) as i32, (size[1] * screen.scale) as i32];
    let usable = [screen.size[0] as i32, screen.size[1] as i32 - (TASKBAR * screen.scale) as i32];
    let centred =
        [screen.origin[0] + (usable[0] - pixels[0]).max(0) / 2, screen.origin[1] + (usable[1] - pixels[1]).max(0) / 2];
    let position = match kept {
        Some((_, p)) => [
            p[0].clamp(screen.origin[0] - 8, (screen.origin[0] + usable[0] - pixels[0]).max(screen.origin[0])),
            p[1].clamp(screen.origin[1], (screen.origin[1] + usable[1] - pixels[1]).max(screen.origin[1])),
        ],
        None => centred,
    };
    Place { size, position }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_HD: Screen = Screen { origin: [0, 0], size: [1920, 1080], scale: 1.0 };
    const SECOND: Screen = Screen { origin: [1920, 0], size: [1920, 1080], scale: 1.0 };

    #[test]
    fn a_first_start_is_centred_and_fits() {
        let p = place(&[FULL_HD], Some([0, 0]), None, None);
        assert!(p.size[0] < 1920.0 && p.size[1] < 1080.0 - TASKBAR);
        assert!(p.size[0] > 1500.0 && p.size[1] > 800.0);
        let right = p.position[0] + p.size[0] as i32;
        assert_eq!(p.position[0], 1920 - right, "same gap on both sides");
        assert!(p.position[1] >= 0 && p.position[1] + p.size[1] as i32 <= 1080 - TASKBAR as i32);
    }

    #[test]
    fn a_saved_size_larger_than_the_screen_is_cut_to_fit() {
        let p = place(&[FULL_HD], Some([0, 0]), Some([2560.0, 1400.0]), Some([-8, -8]));
        assert!(p.size[0] <= 1920.0 - MARGIN && p.size[1] <= 1080.0 - TASKBAR - MARGIN);
        assert!(p.position[0] >= -8 && p.position[1] >= 0);
        assert!(p.position[0] + p.size[0] as i32 <= 1920);
    }

    #[test]
    fn the_place_it_was_left_in_is_kept() {
        let p = place(&[FULL_HD, SECOND], Some([0, 0]), Some([1200.0, 700.0]), Some([2100, 150]));
        assert_eq!(p, Place { size: [1200.0, 700.0], position: [2100, 150] });
    }

    #[test]
    fn a_place_on_a_screen_that_is_gone_goes_back_to_the_first_one() {
        let p = place(&[FULL_HD], Some([0, 0]), Some([1200.0, 700.0]), Some([2500, 150]));
        assert_eq!(p.size, [1200.0, 700.0]);
        assert!(p.position[0] >= 0 && p.position[0] + 1200 <= 1920);
    }

    #[test]
    fn scaled_screens_count_in_their_own_units() {
        let hidpi = Screen { origin: [0, 0], size: [3840, 2160], scale: 2.0 };
        let p = place(&[hidpi], Some([0, 0]), None, None);
        assert!(p.size[0] < 1920.0 && p.size[1] < 1080.0);
        assert!(p.position[0] > 0 && p.position[0] + (p.size[0] * 2.0) as i32 <= 3840);
    }
}
