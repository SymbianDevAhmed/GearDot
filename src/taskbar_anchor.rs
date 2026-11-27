use std::time::{Duration, Instant};

pub fn taskbar_slot_origin(width: i32, cell: i32, slot: usize) -> i32 {
    width - cell * (slot as i32 + 1)
}

#[derive(Default)]
pub struct TaskbarAnchor {
    current: Option<i32>,
    candidate: Option<(i32, Instant)>,
}

impl TaskbarAnchor {
    pub fn position(&mut self, measured: Option<i32>, fallback: i32, now: Instant) -> i32 {
        let Some(current) = self.current else {
            let first = measured.unwrap_or(fallback);
            self.current = Some(first);
            return first;
        };
        let Some(measured) = measured.filter(|&position| position != current) else {
            self.candidate = None;
            return current;
        };
        if let Some((candidate, since)) = self.candidate {
            if candidate == measured {
                if now.duration_since(since) >= Duration::from_millis(350) {
                    self.current = Some(measured);
                    self.candidate = None;
                    return measured;
                }
                return current;
            }
        }
        self.candidate = Some((measured, now));
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_a_device_leaves_existing_icons_at_the_same_screen_coordinates() {
        let right = 1000;
        let cell = 57;
        let screen_x = |count, slot| {
            let width = count * cell;
            right - width + taskbar_slot_origin(width, cell, slot)
        };
        assert_eq!(screen_x(1, 0), screen_x(3, 0));
        assert_eq!(screen_x(2, 1), screen_x(3, 1));
    }

    #[test]
    fn temporary_shell_geometry_preserves_the_visible_anchor() {
        let mut anchor = TaskbarAnchor::default();
        let now = Instant::now();
        assert_eq!(anchor.position(Some(1700), 1600, now), 1700);
        assert_eq!(
            anchor.position(None, 1600, now + Duration::from_millis(120)),
            1700
        );
        assert_eq!(
            anchor.position(Some(1750), 1600, now + Duration::from_millis(240)),
            1700
        );
        assert_eq!(
            anchor.position(Some(1700), 1600, now + Duration::from_millis(360)),
            1700
        );
    }

    #[test]
    fn sustained_shell_geometry_moves_the_anchor() {
        let mut anchor = TaskbarAnchor::default();
        let now = Instant::now();
        assert_eq!(anchor.position(None, 1600, now), 1600);
        assert_eq!(anchor.position(Some(1700), 1600, now), 1600);
        assert_eq!(
            anchor.position(Some(1700), 1600, now + Duration::from_millis(350)),
            1700
        );
    }
}
