//! Small vector icons, painted with lines so they don't depend on font glyphs (which egui
//! draws from its emoji fallback, at a different size and baseline from the text).

use std::f32::consts::TAU;

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Terminal,
    Loop,
    While,
    Bolt,
    File,
    Folder,
    Clock,
    Mountain,
    Play,
    Stop,
    Skip,
    Trash,
    Warning,
    Search,
    More,
    Chevron,
    ChevronUp,
    Close,
    Plus,
    Grip,
    Check,
    Copy,
}

/// Paints `icon` centered in `rect`.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 12.0; // the icons are drawn on a 12×12 grid
    let at = |x: f32, y: f32| pos2(c.x + (x - 6.0) * u, c.y + (y - 6.0) * u);
    let st = Stroke::new((1.15 * u).max(1.0), color);
    let line = |pts: &[Pos2]| {
        p.add(Shape::line(pts.to_vec(), st));
    };
    let fill = |pts: &[Pos2]| {
        p.add(Shape::convex_polygon(pts.to_vec(), color, Stroke::NONE));
    };
    match icon {
        Icon::Terminal => {
            line(&[at(2.5, 3.5), at(5.5, 6.0), at(2.5, 8.5)]);
            line(&[at(6.5, 9.0), at(10.0, 9.0)]);
        }
        Icon::Loop | Icon::While => {
            let r = 3.8 * u;
            let center = at(6.0, 6.0);
            let (a0, a1) = (0.15 * TAU, 0.95 * TAU);
            let point = |i: usize| {
                let a = a0 + (a1 - a0) * i as f32 / 24.0;
                center + vec2(a.cos(), a.sin()) * r
            };
            let end = point(24);
            p.add(Shape::line((0..=24).map(point).collect(), st));
            line(&[
                end + vec2(-2.2 * u, -0.6 * u),
                end,
                end + vec2(0.4 * u, -2.2 * u),
            ]);
            if icon == Icon::While {
                p.circle_filled(center, 1.0 * u, color);
            }
        }
        Icon::Bolt => fill(&[
            at(7.0, 1.5),
            at(3.0, 7.0),
            at(6.0, 7.0),
            at(5.0, 10.5),
            at(9.0, 5.0),
            at(6.0, 5.0),
        ]),
        Icon::File => {
            line(&[
                at(3.0, 1.5),
                at(7.0, 1.5),
                at(9.5, 4.0),
                at(9.5, 10.5),
                at(3.0, 10.5),
                at(3.0, 1.5),
            ]);
            line(&[at(7.0, 1.5), at(7.0, 4.0), at(9.5, 4.0)]);
        }
        Icon::Folder => line(&[
            at(1.5, 3.0),
            at(1.5, 10.0),
            at(10.5, 10.0),
            at(10.5, 4.0),
            at(5.5, 4.0),
            at(4.5, 2.5),
            at(1.5, 2.5),
            at(1.5, 3.0),
        ]),
        Icon::Clock => {
            p.circle_stroke(at(6.0, 6.0), 4.3 * u, st);
            line(&[at(6.0, 3.5), at(6.0, 6.0), at(8.0, 7.2)]);
        }
        Icon::Mountain => {
            fill(&[at(1.0, 10.0), at(5.0, 3.0), at(9.0, 10.0)]);
            fill(&[at(5.5, 10.0), at(8.0, 5.5), at(11.0, 10.0)]);
        }
        Icon::Play => fill(&[at(3.5, 2.0), at(10.0, 6.0), at(3.5, 10.0)]),
        Icon::Stop => {
            p.rect_filled(Rect::from_two_pos(at(2.5, 2.5), at(9.5, 9.5)), u, color);
        }
        Icon::Skip => {
            fill(&[at(2.0, 2.5), at(7.5, 6.0), at(2.0, 9.5)]);
            line(&[at(9.5, 2.5), at(9.5, 9.5)]);
        }
        Icon::Trash => {
            line(&[at(2.0, 3.0), at(10.0, 3.0)]);
            line(&[at(4.5, 3.0), at(4.5, 1.5), at(7.5, 1.5), at(7.5, 3.0)]);
            line(&[at(3.0, 3.0), at(3.5, 10.5), at(8.5, 10.5), at(9.0, 3.0)]);
            line(&[at(5.0, 5.0), at(5.0, 8.5)]);
            line(&[at(7.0, 5.0), at(7.0, 8.5)]);
        }
        Icon::Warning => {
            line(&[at(6.0, 1.5), at(11.0, 10.5), at(1.0, 10.5), at(6.0, 1.5)]);
            line(&[at(6.0, 4.8), at(6.0, 7.3)]);
            p.circle_filled(at(6.0, 9.0), 0.7 * u, color);
        }
        Icon::Search => {
            p.circle_stroke(at(5.0, 5.0), 3.3 * u, st);
            line(&[at(7.5, 7.5), at(10.5, 10.5)]);
        }
        Icon::More => {
            for x in [2.5, 6.0, 9.5] {
                p.circle_filled(at(x, 6.0), 0.95 * u, color);
            }
        }
        Icon::Chevron => line(&[at(3.0, 4.5), at(6.0, 7.5), at(9.0, 4.5)]),
        Icon::ChevronUp => line(&[at(3.0, 7.5), at(6.0, 4.5), at(9.0, 7.5)]),
        Icon::Close => {
            line(&[at(3.5, 3.5), at(8.5, 8.5)]);
            line(&[at(8.5, 3.5), at(3.5, 8.5)]);
        }
        Icon::Plus => {
            line(&[at(6.0, 2.5), at(6.0, 9.5)]);
            line(&[at(2.5, 6.0), at(9.5, 6.0)]);
        }
        Icon::Grip => {
            for x in [4.5, 7.5] {
                for y in [3.0, 6.0, 9.0] {
                    p.circle_filled(at(x, y), 0.85 * u, color);
                }
            }
        }
        Icon::Check => line(&[at(2.5, 6.5), at(5.0, 9.0), at(9.5, 3.5)]),
        Icon::Copy => {
            line(&[
                at(4.0, 4.0),
                at(10.0, 4.0),
                at(10.0, 10.5),
                at(4.0, 10.5),
                at(4.0, 4.0),
            ]);
            line(&[at(2.0, 8.0), at(2.0, 1.5), at(8.0, 1.5)]);
        }
    }
}
