use std::time::{Duration, Instant};

const OPEN: Duration = Duration::from_millis(210);
const CLOSE: Duration = Duration::from_millis(150);
const SMALL: f32 = 0.86;

pub enum WindowMotion {
    Opening(Instant),
    Still,
    Closing { started: Instant, from: f32 },
    Closed,
}

impl WindowMotion {
    pub fn close(&mut self, now: Instant) {
        if !matches!(self, Self::Closing { .. } | Self::Closed) {
            *self = Self::Closing {
                started: now,
                from: self.scale(now),
            };
        }
    }

    pub fn scale(&self, now: Instant) -> f32 {
        match self {
            Self::Opening(start) => {
                let t = progress(*start, now, OPEN);
                SMALL + (1.0 - SMALL) * (1.0 - (1.0 - t).powi(3))
            }
            Self::Closing { started, from } => {
                let t = progress(*started, now, CLOSE);
                from + (SMALL - from) * t.powi(2)
            }
            Self::Still => 1.0,
            Self::Closed => SMALL,
        }
    }

    pub fn advance(&mut self, now: Instant) {
        *self = match *self {
            Self::Opening(start) if now.saturating_duration_since(start) >= OPEN => Self::Still,
            Self::Closing { started, .. } if now.saturating_duration_since(started) >= CLOSE => {
                Self::Closed
            }
            _ => return,
        };
    }

    pub fn running(&self) -> bool {
        matches!(self, Self::Opening(_) | Self::Closing { .. })
    }
}

fn progress(start: Instant, now: Instant, duration: Duration) -> f32 {
    (now.saturating_duration_since(start).as_secs_f32() / duration.as_secs_f32()).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_and_closing_reach_their_final_states() {
        let now = Instant::now();
        let mut motion = WindowMotion::Opening(now);
        assert_eq!(motion.scale(now), SMALL);
        motion.advance(now + OPEN);
        assert_eq!(motion.scale(now + OPEN), 1.0);
        motion.close(now + OPEN);
        assert!(motion.running());
        motion.advance(now + OPEN + CLOSE);
        assert!(matches!(motion, WindowMotion::Closed));
    }

    #[test]
    fn interrupting_opening_or_closing_does_not_restart_the_scale() {
        let now = Instant::now();
        let early_close = now + Duration::from_millis(40);
        let mut motion = WindowMotion::Opening(now);
        let current = motion.scale(early_close);
        motion.close(early_close);
        assert_eq!(motion.scale(early_close), current);
        let repeat = early_close + Duration::from_millis(60);
        let during_close = motion.scale(repeat);
        motion.close(repeat);
        assert_eq!(motion.scale(repeat), during_close);
    }
}
