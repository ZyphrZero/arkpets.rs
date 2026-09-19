use crate::{Direction, DirectionSwitching, PetConfig};

/// Tracks actual pointer motion, independently of a moving pet/window.
#[derive(Default)]
pub struct DirectionGesture {
    previous_x: Option<f32>,
    travel: f32,
    last_direction: Option<Direction>,
    dragging: bool,
}

impl DirectionGesture {
    pub fn start_drag(&mut self, x: f32) {
        *self = Self {
            previous_x: Some(x),
            dragging: true,
            ..Self::default()
        };
    }

    pub fn update(
        &mut self,
        x: Option<f32>,
        hovered: bool,
        config: &PetConfig,
    ) -> Option<Direction> {
        if (self.dragging && !config.allow_drag) || (!self.dragging && !hovered) {
            *self = Self::default();
            return None;
        }
        let x = x.filter(|x| x.is_finite())?;
        if let Some(previous) = self.previous_x.replace(x) {
            let delta = x - previous;
            if delta != 0.0 {
                if self.travel.signum() != delta.signum() {
                    self.travel = 0.0;
                }
                self.travel += delta;
                // Accumulate small movements, but ignore jitter and vertical drags.
                if self.travel.abs() >= 5.0 {
                    let direction = if self.travel < 0.0 {
                        Direction::Left
                    } else {
                        Direction::Right
                    };
                    self.last_direction = Some(direction);
                    self.travel = 0.0;
                    if config.direction_switching == DirectionSwitching::Hover
                        || (self.dragging && config.direction_switching == DirectionSwitching::Drag)
                    {
                        return Some(direction);
                    }
                }
            }
        }
        None
    }

    pub fn release(&mut self, config: &PetConfig) -> Option<Direction> {
        let direction = (self.dragging
            && config.allow_drag
            && config.direction_switching != DirectionSwitching::Disabled)
            .then_some(self.last_direction)
            .flatten();
        *self = Self::default();
        direction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_distinguish_release_drag_and_hover_and_ignore_stationary_input() {
        for mode in [
            DirectionSwitching::Disabled,
            DirectionSwitching::DragRelease,
            DirectionSwitching::Drag,
            DirectionSwitching::Hover,
        ] {
            let config = PetConfig {
                direction_switching: mode,
                ..Default::default()
            };
            let mut gesture = DirectionGesture::default();
            gesture.start_drag(50.0);
            let during = gesture.update(Some(30.0), true, &config);
            assert_eq!(
                during,
                matches!(mode, DirectionSwitching::Drag | DirectionSwitching::Hover)
                    .then_some(Direction::Left)
            );
            assert_eq!(gesture.update(Some(30.0), true, &config), None);
            assert_eq!(
                gesture.release(&config),
                (mode != DirectionSwitching::Disabled).then_some(Direction::Left)
            );
            assert_eq!(gesture.update(Some(30.0), true, &config), None);
            assert_eq!(
                gesture.update(Some(50.0), true, &config),
                (mode == DirectionSwitching::Hover).then_some(Direction::Right)
            );
        }
    }

    #[test]
    fn jitter_hover_reentry_and_disabled_drag_do_not_flip() {
        let mut config = PetConfig {
            direction_switching: DirectionSwitching::Hover,
            ..Default::default()
        };
        let mut gesture = DirectionGesture::default();
        for x in [10.0, 11.0, 10.0, 11.0, 10.0] {
            assert_eq!(gesture.update(Some(x), true, &config), None);
        }
        gesture.update(None, false, &config);
        assert_eq!(gesture.update(Some(100.0), true, &config), None);
        config.allow_drag = false;
        gesture.start_drag(100.0);
        assert_eq!(gesture.update(Some(-100.0), true, &config), None);
        assert_eq!(gesture.release(&config), None);
    }

    #[test]
    fn disabling_interaction_animation_does_not_disable_direction_gestures() {
        let config = PetConfig {
            allow_interact: false,
            direction_switching: DirectionSwitching::Drag,
            ..Default::default()
        };
        let mut gesture = DirectionGesture::default();
        gesture.start_drag(100.0);
        assert_eq!(
            gesture.update(Some(80.0), true, &config),
            Some(Direction::Left)
        );
        assert_eq!(gesture.release(&config), Some(Direction::Left));
    }
}
