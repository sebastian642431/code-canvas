pub const COLOURS: [[u8; 3]; 6] =
    [[242, 242, 242], [255, 92, 92], [255, 196, 61], [84, 214, 128], [78, 164, 255], [190, 140, 255]];
pub const COLOUR_NAMES: [&str; 6] = ["White", "Red", "Yellow", "Green", "Blue", "Violet"];
pub const WIDTHS: [f32; 3] = [2.5, 5.0, 10.0];
pub const WIDTH_NAMES: [&str; 3] = ["Fine", "Medium", "Thick"];
pub const RUBBER: f32 = 14.0;
const APART: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pen {
    pub on: bool,
    pub rubbing: bool,
    pub colour: usize,
    pub width: usize,
}

impl Default for Pen {
    fn default() -> Self {
        Pen { on: false, rubbing: false, colour: 1, width: 1 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    pub colour: [u8; 3],
    pub width: f32,
}

fn apart(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn from_stretch(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (along, to) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let long = along[0] * along[0] + along[1] * along[1];
    let t = if long > 0.0 { ((to[0] * along[0] + to[1] * along[1]) / long).clamp(0.0, 1.0) } else { 0.0 };
    apart(p, [a[0] + along[0] * t, a[1] + along[1] * t])
}

impl Stroke {
    pub fn begin(at: [f32; 2], pen: Pen, zoom: f32) -> Stroke {
        let colour = COLOURS[pen.colour.min(COLOURS.len() - 1)];
        Stroke { points: vec![at, at], colour, width: WIDTHS[pen.width.min(WIDTHS.len() - 1)] / zoom }
    }

    pub fn is_dot(&self) -> bool {
        self.points.len() == 2 && self.points[0] == self.points[1]
    }

    pub fn reach(&mut self, at: [f32; 2], zoom: f32) -> bool {
        let Some(last) = self.points.last().copied() else { return false };
        if apart(last, at) < APART / zoom {
            return false;
        }
        match self.is_dot() {
            true => self.points[1] = at,
            false => self.points.push(at),
        }
        true
    }

    fn touched(&self, at: [f32; 2], near: f32) -> bool {
        self.points.windows(2).any(|pair| from_stretch(at, pair[0], pair[1]) <= near)
    }
}

pub fn rub(strokes: &mut Vec<Stroke>, at: [f32; 2], reach: f32) -> bool {
    const PIECES: usize = 4096;
    let mut left = Vec::with_capacity(strokes.len());
    let mut rubbed = false;
    for stroke in strokes.drain(..) {
        let near = reach + stroke.width * 0.5;
        if !stroke.touched(at, near) {
            left.push(stroke);
            continue;
        }
        rubbed = true;
        let step = (near * 0.5).max(f32::EPSILON);
        let mut close = Vec::with_capacity(stroke.points.len());
        for pair in stroke.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            close.push(a);
            let long = apart(a, b);
            if long > step && from_stretch(at, a, b) <= near {
                let pieces = ((long / step).ceil() as usize).min(PIECES);
                let between = (1..pieces).map(|nth| nth as f32 / pieces as f32);
                close.extend(between.map(|t| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]));
            }
        }
        close.extend(stroke.points.last());
        let mut piece: Vec<[f32; 2]> = Vec::new();
        let mut keep = |piece: &mut Vec<[f32; 2]>| {
            if piece.len() >= 2 {
                left.push(Stroke { points: std::mem::take(piece), colour: stroke.colour, width: stroke.width });
            }
            piece.clear();
        };
        for point in close {
            match apart(at, point) <= near {
                true => keep(&mut piece),
                false => piece.push(point),
            }
        }
        keep(&mut piece);
    }
    *strokes = left;
    rubbed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: &[[f32; 2]]) -> Stroke {
        Stroke { points: points.to_vec(), colour: COLOURS[0], width: 2.0 }
    }

    #[test]
    fn a_stroke_grows_only_when_the_pen_has_gone_somewhere() {
        let pen = Pen { on: true, ..Default::default() };
        let mut stroke = Stroke::begin([10.0, 10.0], pen, 2.0);
        assert!(stroke.is_dot());
        assert_eq!((stroke.width, stroke.colour), (WIDTHS[1] / 2.0, COLOURS[1]));
        assert!(!stroke.reach([10.2, 10.0], 2.0), "less than a pixel and a half on screen");
        assert!(stroke.reach([14.0, 10.0], 2.0));
        assert_eq!(stroke.points, vec![[10.0, 10.0], [14.0, 10.0]]);
        assert!(stroke.reach([14.0, 20.0], 2.0));
        assert_eq!(stroke.points.len(), 3);
    }

    #[test]
    fn rubbing_the_middle_of_a_stroke_leaves_its_two_ends() {
        let points: Vec<[f32; 2]> = (0..=100).map(|x| [x as f32, 0.0]).collect();
        let mut strokes = vec![line(&points), line(&[[0.0, 50.0], [100.0, 50.0]])];
        assert!(rub(&mut strokes, [50.0, 0.0], 5.0));
        assert_eq!(strokes.len(), 3, "two ends, and the other stroke whole");
        assert_eq!(strokes[2].points, vec![[0.0, 50.0], [100.0, 50.0]]);
        let (before, after) = (&strokes[0].points, &strokes[1].points);
        assert_eq!((before[0], *before.last().unwrap()), ([0.0, 0.0], [43.0, 0.0]));
        assert_eq!((after[0], *after.last().unwrap()), ([57.0, 0.0], [100.0, 0.0]));
        assert!(!rub(&mut strokes, [50.0, 25.0], 5.0), "nothing there");
        assert_eq!(strokes.len(), 3);
    }

    #[test]
    fn a_long_stretch_loses_only_what_is_rubbed() {
        let mut strokes = vec![line(&[[0.0, 0.0], [1000.0, 0.0]])];
        assert!(rub(&mut strokes, [500.0, 2.0], 9.0));
        assert_eq!(strokes.len(), 2);
        let (left, right) = (strokes[0].points.last().unwrap()[0], strokes[1].points[0][0]);
        assert!((485.0..491.0).contains(&left) && (509.0..515.0).contains(&right), "{left} {right}");
        assert_eq!((strokes[0].points[0], *strokes[1].points.last().unwrap()), ([0.0, 0.0], [1000.0, 0.0]));
    }

    #[test]
    fn a_dot_is_rubbed_out_whole_and_an_end_is_shortened() {
        let mut strokes = vec![line(&[[5.0, 5.0], [5.0, 5.0]])];
        assert!(strokes[0].is_dot());
        assert!(rub(&mut strokes, [6.0, 5.0], 3.0));
        assert!(strokes.is_empty());
        let points: Vec<[f32; 2]> = (0..=20).map(|x| [x as f32, 0.0]).collect();
        let mut strokes = vec![line(&points)];
        assert!(rub(&mut strokes, [0.0, 0.0], 4.0));
        assert_eq!(strokes.len(), 1);
        assert_eq!((strokes[0].points[0], strokes[0].points.len()), ([6.0, 0.0], 15));
    }
}
