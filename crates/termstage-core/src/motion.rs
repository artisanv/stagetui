//! Motion: entrance effects (`fade`, `slide-up`, `slide-left`, `pop`) and the
//! looping `pulse`.
//!
//! Effects are pure functions of time, so a headless run with a virtual clock
//! produces exactly the same frames every time.

use std::time::Duration;

use crate::{Error, Result};

/// Allowed `animate=` values.
pub const ANIMATIONS: [&str; 6] = ["none", "fade", "slide-up", "slide-left", "pulse", "pop"];

/// How long an entrance effect lasts.
pub const ENTER: Duration = Duration::from_millis(320);
/// One full `pulse` cycle.
pub const PULSE_PERIOD: Duration = Duration::from_millis(1800);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anim {
    #[default]
    None,
    Fade,
    SlideUp,
    SlideLeft,
    Pulse,
    Pop,
}

impl Anim {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "none" => Self::None,
            "fade" => Self::Fade,
            "slide-up" => Self::SlideUp,
            "slide-left" => Self::SlideLeft,
            "pulse" => Self::Pulse,
            "pop" => Self::Pop,
            other => {
                return Err(Error::friendly(
                    format!("Unknown animation animate=\"{other}\""),
                    "animations are a fixed, curated set",
                    format!("use one of: {}", ANIMATIONS.join(", ")),
                ));
            }
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fade => "fade",
            Self::SlideUp => "slide-up",
            Self::SlideLeft => "slide-left",
            Self::Pulse => "pulse",
            Self::Pop => "pop",
        }
    }

    /// Whether this animation never ends.
    pub fn loops(self) -> bool {
        self == Self::Pulse
    }
}

/// How to draw a widget right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effect {
    /// Offset in cells.
    pub dx: i32,
    pub dy: i32,
    /// 0 = invisible (background color), 1 = normal.
    pub alpha: f32,
    /// Extra accent brightness on borders, 0..1.
    pub glow: f32,
    /// Whether the entrance has finished (pictures are shown only then).
    pub settled: bool,
}

impl Effect {
    pub const NONE: Self = Self {
        dx: 0,
        dy: 0,
        alpha: 1.0,
        glow: 0.0,
        settled: true,
    };

    /// Whether drawing differs from a plain paint.
    pub fn is_identity(&self) -> bool {
        self.dx == 0 && self.dy == 0 && self.alpha >= 1.0
    }
}

/// Smooth deceleration.
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// The effect of `anim` on a widget that appeared `age` ago, at absolute time `now`.
pub fn effect(anim: Anim, age: Duration, now: Duration, reduce_motion: bool) -> Effect {
    if reduce_motion {
        return Effect::NONE;
    }
    let t = (age.as_secs_f32() / ENTER.as_secs_f32()).clamp(0.0, 1.0);
    let e = ease_out_cubic(t);
    let settled = t >= 1.0;
    match anim {
        Anim::None => Effect::NONE,
        Anim::Fade => Effect {
            alpha: e,
            settled,
            ..Effect::NONE
        },
        Anim::SlideUp => Effect {
            dy: ((1.0 - e) * 3.0).round() as i32,
            alpha: e,
            settled,
            ..Effect::NONE
        },
        Anim::SlideLeft => Effect {
            dx: ((1.0 - e) * 8.0).round() as i32,
            alpha: e,
            settled,
            ..Effect::NONE
        },
        Anim::Pop => Effect {
            alpha: 0.35 + 0.65 * e,
            glow: 1.0 - e,
            settled,
            ..Effect::NONE
        },
        Anim::Pulse => {
            let phase = (now.as_secs_f32() / PULSE_PERIOD.as_secs_f32()) * std::f32::consts::TAU;
            Effect {
                glow: 0.5 - 0.5 * phase.cos(),
                ..Effect::NONE
            }
        }
    }
}

/// Whether `anim` still needs frames at `age`.
pub fn active(anim: Anim, age: Duration, reduce_motion: bool) -> bool {
    !reduce_motion && (anim.loops() || (anim != Anim::None && age < ENTER))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn entrance_effects_settle() {
        for a in [Anim::Fade, Anim::SlideUp, Anim::SlideLeft, Anim::Pop] {
            let start = effect(a, MS(0), MS(0), false);
            assert!(!start.settled && start.alpha < 1.0, "{a:?}");
            let end = effect(a, MS(400), MS(400), false);
            assert!(end.settled && end.is_identity(), "{a:?} {end:?}");
            assert!(!active(a, MS(400), false));
        }
        assert_eq!(effect(Anim::SlideUp, MS(0), MS(0), false).dy, 3);
        assert_eq!(effect(Anim::SlideLeft, MS(0), MS(0), false).dx, 8);
    }

    #[test]
    fn deterministic_and_reducible() {
        let a = effect(Anim::SlideUp, MS(120), MS(120), false);
        let b = effect(Anim::SlideUp, MS(120), MS(120), false);
        assert_eq!(a, b);
        assert_eq!(effect(Anim::SlideUp, MS(0), MS(0), true), Effect::NONE);
        assert!(!active(Anim::Pulse, MS(0), true));
    }

    #[test]
    fn pulse_loops() {
        assert!(active(Anim::Pulse, MS(99_999), false));
        let low = effect(Anim::Pulse, MS(0), MS(0), false).glow;
        let high = effect(Anim::Pulse, MS(900), MS(900), false).glow;
        assert!(low < 0.01 && high > 0.99, "{low} {high}");
    }

    #[test]
    fn names_round_trip() {
        for n in ANIMATIONS {
            assert_eq!(Anim::parse(n).unwrap().name(), n);
        }
        assert!(
            Anim::parse("spin")
                .unwrap_err()
                .to_string()
                .contains("fix:")
        );
    }
}
