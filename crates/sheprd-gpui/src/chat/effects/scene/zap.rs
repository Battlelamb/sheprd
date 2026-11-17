//! Zap: the message's letters line up across the top of the pane and sway
//! as a formation, while three pixel ships slide along the bottom and shoot
//! them down one by one; each letter hit bursts into pixels. Each row of
//! the formation takes its own bright color.

use super::{BRIGHTS, Hue, Mark, Rect, Words, between, envelope, unit};
use std::f32::consts::TAU;

/// Letters in the formation; a longer message lends only its first ones.
pub(crate) const MAX_TARGETS: usize = 24;
/// Letters in a row of the formation.
pub(crate) const ROW: usize = 12;
/// How far apart the letters stand, and their rows.
const COLUMN: f32 = 30.;
const LINE: f32 = 38.;
/// The letters' size.
const LETTER: f32 = 22.;
/// How long a shot takes to reach its letter.
const FLIGHT: f32 = 0.5;
/// How long a letter's burst lasts.
const BURST: f32 = 0.7;
/// When the first and the last letters are hit.
const FIRST_HIT: f32 = 0.8;
const LAST_HIT: f32 = 3.6;
const SHIPS: usize = 3;
/// A ship, drawn in pixels: a cannon on a wide hull.
const SHIP: [&str; 4] = ["..#..", "..#..", ".###.", "#####"];
/// The side of one of a ship's pixels, and of a burst's.
const PIXEL: f32 = 4.;

/// Where letter `index` of `count` stands at `t`: its center.
fn letter_at(index: usize, count: usize, t: f32, w: f32, h: f32) -> (f32, f32) {
    let per_row = count.clamp(1, ROW);
    let (row, column) = (index / per_row, index % per_row);
    let left = w / 2. - (per_row - 1) as f32 * COLUMN / 2.;
    let sway = (t * 1.6).sin() * 40.;
    (
        left + column as f32 * COLUMN + sway,
        h * 0.12 + row as f32 * LINE + 6. * t + LETTER / 2.,
    )
}

/// Where ship `index`'s cannon is at `t`.
fn ship_at(index: usize, t: f32, w: f32, h: f32) -> (f32, f32) {
    let home = w * (index as f32 + 1.) / (SHIPS as f32 + 1.);
    (
        home + (t * 2. + index as f32 * 2.1).sin() * 60.,
        h - 12. - SHIP.len() as f32 * PIXEL,
    )
}

/// When letter `index` is hit: each at its own moment, all before the end.
fn hit_time(index: usize) -> f32 {
    between(index as u32, 41, FIRST_HIT, LAST_HIT)
}

pub(super) fn zap(t: f32, length: f32, w: f32, h: f32, letters: usize, marks: &mut Vec<Mark>) {
    let fade = envelope(t, length, 0.2, 0.4);
    // The pane dims, so the formation reads over the conversation.
    marks.push(Mark::Veil {
        alpha: 0.6 * fade,
        hole: None,
    });
    let count = letters.min(MAX_TARGETS);
    for index in 0..count {
        let hit = hit_time(index);
        let shooter = index % SHIPS;
        if t < hit {
            let (x, y) = letter_at(index, count, t, w, h);
            marks.push(Mark::Words {
                x,
                y: y - LETTER / 2.,
                size: LETTER,
                alpha: fade,
                words: Words::Letter(index),
            });
        }
        let fired = hit - FLIGHT;
        if (fired..hit).contains(&t) {
            // From where the ship was when it fired to where the letter is
            // when it is hit.
            let k = (t - fired) / FLIGHT;
            let (sx, sy) = ship_at(shooter, fired, w, h);
            let (lx, ly) = letter_at(index, count, hit, w, h);
            let (x, y) = (sx + (lx - sx) * k, sy + (ly - sy) * k);
            marks.push(Mark::Box {
                rect: Rect {
                    x: x - 1.5,
                    y: y - 5.,
                    w: 3.,
                    h: 10.,
                },
                round: 1.,
                hue: Hue::White,
                alpha: fade,
            });
        }
        let since = t - hit;
        if (0. ..BURST).contains(&since) {
            let (x, y) = letter_at(index, count, hit, w, h);
            let flash = 1. - since / 0.15;
            if flash > 0. {
                marks.push(Mark::Box {
                    rect: Rect {
                        x: x - 9.,
                        y: y - 9.,
                        w: 18.,
                        h: 18.,
                    },
                    round: 2.,
                    hue: Hue::White,
                    alpha: flash * fade,
                });
            }
            for piece in 0..12 {
                let id = (index * 12 + piece) as u32;
                let angle = unit(id, 43) * TAU;
                let speed = between(id, 44, 60., 170.);
                marks.push(Mark::Box {
                    rect: Rect {
                        x: x + angle.cos() * speed * since - PIXEL / 2.,
                        y: y + angle.sin() * speed * since + 60. * since * since - PIXEL / 2.,
                        w: PIXEL,
                        h: PIXEL,
                    },
                    round: 0.,
                    hue: Hue::Bright(index % BRIGHTS),
                    alpha: (1. - since / BURST) * fade,
                });
            }
        }
    }
    for ship in 0..SHIPS {
        let (x, top) = ship_at(ship, t, w, h);
        let half = SHIP[0].len() as f32 * PIXEL / 2.;
        for (row, line) in SHIP.iter().enumerate() {
            for (column, _) in line.char_indices().filter(|(_, pixel)| *pixel == '#') {
                marks.push(Mark::Box {
                    rect: Rect {
                        x: x - half + column as f32 * PIXEL,
                        y: top + row as f32 * PIXEL,
                        w: PIXEL,
                        h: PIXEL,
                    },
                    round: 0.,
                    hue: Hue::Bright(1),
                    alpha: fade,
                });
            }
        }
    }
}
