use crate::app::App;
use crate::theme::*;
use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::Color;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tachyonfx::{CellFilter, Effect, Interpolation::*, fx};

pub const DISMISS_MS: u32 = 320;
const GLOW: Color = Color::Rgb(0x1d, 0x3a, 0x37);
pub const NEW: Color = Color::Rgb(0x25, 0x4d, 0x48);
pub const ALARM: Color = Color::Rgb(0x6b, 0x26, 0x26);

/// Animations the app asks for; they need the rects from the frame being drawn.
pub enum Cue {
    Focus,
    Select,
    Dismiss,
}

#[derive(Default, Clone, Copy)]
pub struct Rects {
    pub panels: [Rect; 3],
    /// The selected row in the focused panel, if it's on screen.
    pub row: Option<Rect>,
}

pub fn apply(f: &mut Frame, app: &mut App) {
    let r = app.rects;
    let cued = !app.cues.is_empty();
    for cue in std::mem::take(&mut app.cues) {
        match cue {
            Cue::Focus => app.fx.add_unique_effect("focus", focus(r.panels[app.focus])),
            Cue::Select => {
                if let Some(row) = r.row {
                    app.fx.add_unique_effect("select", select(row));
                }
            }
            Cue::Dismiss => {
                if let Some(row) = r.row {
                    app.fx.add_unique_effect("dismiss", dismiss(row));
                }
            }
        }
    }
    // After an idle poll the gap can be 250ms; don't let new effects skip ahead by that.
    let mut dt = app.last_frame.elapsed();
    if cued {
        dt = dt.min(Duration::from_millis(16));
    }
    app.last_frame = Instant::now();
    let area = f.area();
    app.fx.process_effects(dt.into(), f.buffer_mut(), area);
}

fn focus(panel: Rect) -> Effect {
    fx::fade_from_fg(TEAL, (450, QuadOut)).with_area(panel).with_filter(CellFilter::Outer(Margin::new(1, 1)))
}

fn select(row: Rect) -> Effect {
    fx::fade_from(TEAL, GLOW, (240, QuadOut)).with_area(row)
}

fn dismiss(row: Rect) -> Effect {
    fx::parallel(&[
        fx::dissolve((DISMISS_MS, QuadIn)).with_area(row),
        fx::fade_to(BG, BG, (DISMISS_MS, QuadIn)).with_area(row),
    ])
}

/// A row background that lights up and fades: new items, deadline crossings.
pub struct Glow {
    at: Instant,
    color: Color,
    ms: u64,
    pulses: u32,
}

impl Glow {
    pub fn new(color: Color, ms: u64, pulses: u32) -> Glow {
        Glow { at: Instant::now(), color, ms, pulses }
    }

    pub fn live(&self) -> bool {
        self.at.elapsed().as_millis() < self.ms as u128
    }

    /// `base` with the glow mixed in for this frame.
    pub fn over(&self, base: Color) -> Color {
        let t = self.at.elapsed().as_millis() as f32 / self.ms as f32;
        if t >= 1.0 {
            return base;
        }
        let fade = (1.0 - t).powi(2);
        let beat = if self.pulses > 1 { (1.0 - (t * self.pulses as f32 * std::f32::consts::TAU).cos()) / 2.0 } else { 1.0 };
        lerp(base, self.color, fade * beat)
    }
}

/// 0..1 sine wave over `period_ms`, from the wall clock so every caller agrees.
pub fn wave(period_ms: u64) -> f32 {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let phase = (ms % period_ms) as f32 / period_ms as f32;
    (1.0 - (phase * std::f32::consts::TAU).cos()) / 2.0
}

pub fn lerp(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
            Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
        }
        _ => a,
    }
}
