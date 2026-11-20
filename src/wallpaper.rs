//! Card looks: one motif repeated on a lattice over a pastel, and a typeface
//! for the name, all picked from a seed.

use std::f32::consts::{FRAC_PI_4, FRAC_PI_8, SQRT_2, TAU};

use dfui::{Color, Font, PaintCx, Path, Rect, Shape};

/// Pastel grounds, each with the deeper tone of itself its motif is drawn in.
const PALETTE: [(u32, u32); 8] = [
    (0xf9d7df, 0xefb3c3), // blush
    (0xfce0cc, 0xf2be98), // peach
    (0xfaf0c0, 0xebd889), // butter
    (0xd2eedb, 0xa6d9b6), // mint
    (0xd3e6f7, 0xa9cbeb), // sky
    (0xe3dbf5, 0xc3b3e8), // lavender
    (0xf1d7ec, 0xddb0d3), // lilac
    (0xe0e9d3, 0xbfd1a6), // sage
];

/// Text color on every card; dark enough for every ground and motif tone.
pub const INK: Color = Color::hex(0x2b2233);

const MOTIFS: [&[u8]; 8] = [
    include_bytes!("../assets/icons/motifs/star.path"),
    include_bytes!("../assets/icons/motifs/heart.path"),
    include_bytes!("../assets/icons/motifs/leaf.path"),
    include_bytes!("../assets/icons/motifs/cloud.path"),
    include_bytes!("../assets/icons/motifs/moon.path"),
    include_bytes!("../assets/icons/motifs/drop.path"),
    include_bytes!("../assets/icons/motifs/sparkle.path"),
    include_bytes!("../assets/icons/motifs/dot.path"),
];

/// Name typefaces, each with a size factor that evens out how big they look.
static FONTS: [(Font, f32); 8] = [
    (Font::BOLD, 1.0),
    (
        Font(include_bytes!("../assets/fonts/AbrilFatface-Regular.ttf")),
        1.05,
    ),
    (
        Font(include_bytes!("../assets/fonts/Pacifico-Regular.ttf")),
        1.0,
    ),
    (
        Font(include_bytes!("../assets/fonts/Lobster-Regular.ttf")),
        1.15,
    ),
    (
        Font(include_bytes!("../assets/fonts/Righteous-Regular.ttf")),
        1.05,
    ),
    (
        Font(include_bytes!("../assets/fonts/Sniglet-ExtraBold.ttf")),
        1.1,
    ),
    (
        Font(include_bytes!("../assets/fonts/CherrySwash-Bold.ttf")),
        1.05,
    ),
    (
        Font(include_bytes!("../assets/fonts/KaushanScript-Regular.ttf")),
        1.15,
    ),
];

/// Rotations snap to this many steps, so a card keeps few rotated shapes.
const TURNS: usize = 24;
/// Lattices are laid out for cards up to this many widths tall.
const MAX_ASPECT: f32 = 2.5;

/// A motif centred at `x`, `y`, `s` wide, in card widths, rotated `turn` steps.
struct Icon {
    x: f32,
    y: f32,
    s: f32,
    turn: usize,
}

pub struct Wallpaper {
    pub ground: Color,
    pub font: Font,
    /// Multiplies the name's text size.
    pub scale: f32,
    tone: Color,
    icons: Vec<Icon>,
    /// The motif at each rotation the icons use, in a view box `SQRT_2` times
    /// the motif's so turned corners stay inside.
    shapes: Vec<Option<Shape>>,
}

impl Wallpaper {
    pub fn new(seed: u64) -> Wallpaper {
        let mut rng = Rng(seed);
        let (ground, tone) = PALETTE[rng.below(PALETTE.len())];
        let motif = Shape::from_bytes(MOTIFS[rng.below(MOTIFS.len())]).expect("bundled motif");
        let (font, scale) = FONTS[rng.below(FONTS.len())];
        let icons = lattice(rng.below(6), &mut rng);
        let mut shapes = vec![None; TURNS];
        for i in &icons {
            shapes[i.turn].get_or_insert_with(|| turned(&motif, i.turn));
        }
        Wallpaper {
            ground: Color::hex(ground),
            font,
            scale,
            tone: Color::hex(tone),
            icons,
            shapes,
        }
    }

    /// Fills `r`, a box with corners of `radius`, with the ground and motifs.
    pub fn paint(&self, cx: &mut PaintCx, r: Rect, radius: f32) {
        cx.fill(r, radius, self.ground);
        let mask = rounded(r, radius);
        cx.with_clip(r, |cx| {
            for i in &self.icons {
                let s = i.s * r.w * SQRT_2;
                let (x, y) = (r.x + i.x * r.w - s / 2.0, r.y + i.y * r.w - s / 2.0);
                let b = Rect::new(x, y, s, s);
                let seen = r.intersect(&b);
                if seen.w * seen.h == 0.0 {
                    continue;
                }
                let shape = self.shapes[i.turn].as_ref().expect("built in new");
                if in_corner(b, r, radius) {
                    cx.path_masked(&placed(shape, b), &mask, self.tone);
                } else {
                    cx.shape(shape, b, self.tone);
                }
            }
        });
    }
}

fn lattice(kind: usize, rng: &mut Rng) -> Vec<Icon> {
    let flat = |x, y, s| Icon { x, y, s, turn: 0 };
    match kind {
        // Uniform grid.
        0 => {
            let g = 1.0 / 4.2;
            rows(g, false)
                .map(|(_, _, x, y)| flat(x + g / 2.0, y, g * 0.42))
                .collect()
        }
        // Every other row shifted by half a step.
        1 => {
            let g = 1.0 / 4.5;
            rows(g, true)
                .map(|(_, _, x, y)| flat(x, y, g * 0.45))
                .collect()
        }
        // Big and small icons alternating like a checkerboard.
        2 => {
            let g = 1.0 / 5.0;
            rows(g, false)
                .map(|(i, j, x, y)| {
                    flat(
                        x + g / 2.0,
                        y,
                        g * if (i + j) % 2 == 0 { 0.62 } else { 0.26 },
                    )
                })
                .collect()
        }
        // A grid tilted 15–45° either way, the icons turned with it.
        3 => {
            let g = 1.0 / 4.0;
            let turn = 1 + rng.below(3);
            let turn = if rng.below(2) == 0 {
                turn
            } else {
                TURNS - turn
            };
            let a = turn as f32 * TAU / TURNS as f32;
            let (sin, cos) = a.sin_cos();
            let (cx, cy, d) = (0.5, MAX_ASPECT / 2.0, 1.0f32.hypot(MAX_ASPECT));
            let n = (d / g).ceil() as i32;
            let mut out = Vec::new();
            for j in -n..=n {
                for i in -n..=n {
                    let (u, v) = (i as f32 * g, j as f32 * g);
                    let (x, y) = (cx + u * cos - v * sin, cy + u * sin + v * cos);
                    if (-g..1.0 + g).contains(&x) && (-g..MAX_ASPECT + g).contains(&y) {
                        out.push(Icon {
                            x,
                            y,
                            s: g * 0.42,
                            turn,
                        });
                    }
                }
            }
            out
        }
        // Offset rows with each icon nudged, resized and tipped a little.
        4 => {
            let g = 1.0 / 3.6;
            rows(g, true)
                .map(|(_, _, x, y)| Icon {
                    x: x + (rng.unit() - 0.5) * g * 0.3,
                    y: y + (rng.unit() - 0.5) * g * 0.3,
                    s: g * (0.25 + rng.unit() * 0.45),
                    turn: (TURNS - 2 + rng.below(5)) % TURNS,
                })
                .collect()
        }
        // Offset rows jittered hard, any rotation.
        _ => {
            let g = 1.0 / 4.0;
            rows(g, true)
                .map(|(_, _, x, y)| Icon {
                    x: x + (rng.unit() - 0.5) * g * 0.7,
                    y: y + (rng.unit() - 0.5) * g * 0.7,
                    s: g * (0.3 + rng.unit() * 0.2),
                    turn: rng.below(TURNS),
                })
                .collect()
        }
    }
}

/// Grid points `g` apart covering the card with a step of margin: column,
/// row, x, y. `offset` shifts odd rows by half a step.
fn rows(g: f32, offset: bool) -> impl Iterator<Item = (usize, usize, f32, f32)> {
    let (cols, rows) = (
        (1.0 / g).ceil() as usize + 2,
        (MAX_ASPECT / g).ceil() as usize + 2,
    );
    (0..rows).flat_map(move |j| {
        let shift = if offset && j % 2 == 1 { g / 2.0 } else { 0.0 };
        (0..cols).map(move |i| (i, j, -g + shift + i as f32 * g, -g / 2.0 + j as f32 * g))
    })
}

/// `motif` rotated `turn` steps about its centre.
fn turned(motif: &Shape, turn: usize) -> Shape {
    let v = motif.view();
    let (cx, cy) = (v.x + v.w / 2.0, v.y + v.h / 2.0);
    let (sin, cos) = (turn as f32 * TAU / TURNS as f32).sin_cos();
    let mut path = motif.path().clone();
    for p in path.contours.iter_mut().flatten().flatten() {
        let (dx, dy) = (p[0] - cx, p[1] - cy);
        *p = [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos];
    }
    let half = v.w.max(v.h) / 2.0 * SQRT_2;
    Shape::new(
        path,
        Rect::new(cx - half, cy - half, 2.0 * half, 2.0 * half),
    )
}

/// `shape`'s outline with its view box mapped onto `b`.
fn placed(shape: &Shape, b: Rect) -> Path {
    let v = shape.view();
    let k = b.w / v.w;
    let mut path = shape.path().clone();
    for p in path.contours.iter_mut().flatten().flatten() {
        *p = [b.x + (p[0] - v.x) * k, b.y + (p[1] - v.y) * k];
    }
    path
}

/// Whether `b` reaches into one of `r`'s rounded corners.
fn in_corner(b: Rect, r: Rect, radius: f32) -> bool {
    let near = |lo: f32, len: f32, blo: f32, blen: f32| {
        blo < lo + radius || blo + blen > lo + len - radius
    };
    near(r.x, r.w, b.x, b.w) && near(r.y, r.h, b.y, b.h)
}

/// `r` with corners of `radius`, each corner two quadratic arcs.
fn rounded(r: Rect, radius: f32) -> Path {
    let line = |a: [f32; 2], b: [f32; 2]| [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b];
    let corners = [
        (r.x + r.w - radius, r.y + radius, -2.0),
        (r.x + r.w - radius, r.y + r.h - radius, 0.0),
        (r.x + radius, r.y + r.h - radius, 2.0),
        (r.x + radius, r.y + radius, 4.0),
    ];
    let at = |cx: f32, cy: f32, a: f32, d: f32| [cx + d * a.cos(), cy + d * a.sin()];
    let mut quads = Vec::new();
    let mut last = None;
    for (cx, cy, start) in corners {
        let a0 = start * FRAC_PI_4;
        let first = at(cx, cy, a0, radius);
        if let Some(p) = last {
            quads.push(line(p, first));
        }
        for k in 0..2 {
            let a = a0 + k as f32 * FRAC_PI_4;
            let ctrl = at(cx, cy, a + FRAC_PI_8, radius / FRAC_PI_8.cos());
            quads.push([
                at(cx, cy, a, radius),
                ctrl,
                at(cx, cy, a + FRAC_PI_4, radius),
            ]);
        }
        last = Some(at(cx, cy, a0 + 2.0 * FRAC_PI_4, radius));
    }
    quads.push(line(last.unwrap(), quads[0][0]));
    Path {
        contours: vec![quads],
    }
}

/// splitmix64: small, and the same sequence on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// In `0.0..1.0`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(rgb: u32) -> f32 {
        let lin = |c: u32| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(rgb >> 16 & 0xff) + 0.7152 * lin(rgb >> 8 & 0xff) + 0.0722 * lin(rgb & 0xff)
    }

    #[test]
    fn ink_reads_on_every_ground_and_motif() {
        let ink = luminance(0x2b2233);
        for (ground, tone) in PALETTE {
            for c in [ground, tone] {
                let ratio = (luminance(c) + 0.05) / (ink + 0.05);
                assert!(ratio >= 7.0, "{c:06x}: {ratio}");
            }
        }
    }

    #[test]
    fn every_seed_covers_the_card() {
        for seed in 0..500 {
            let w = Wallpaper::new(seed);
            assert!(
                w.icons.iter().any(|i| i.x < 0.2 && i.y < 0.2),
                "seed {seed}"
            );
            assert!(
                w.icons.iter().any(|i| i.x > 0.8 && i.y > MAX_ASPECT - 0.2),
                "seed {seed}"
            );
            assert!(w.icons.iter().all(|i| w.shapes[i.turn].is_some()));
        }
    }

    #[test]
    fn rounded_rect_is_closed() {
        let q = &rounded(Rect::new(0.0, 0.0, 100.0, 150.0), 20.0).contours[0];
        for (a, b) in q.iter().zip(q.iter().cycle().skip(1)) {
            assert!((a[2][0] - b[0][0]).abs() < 1e-3 && (a[2][1] - b[0][1]).abs() < 1e-3);
        }
    }
}
