use arkpets::config::EasingFunction;
use std::time::Instant;

/// Each viewport keeps its own visual position and facing while sharing one pose.
#[derive(Default)]
pub struct Presentation {
    position: Option<Transition<2>>,
    facing: Option<Transition<1>>,
}

impl Presentation {
    pub fn position(
        &mut self,
        target: eframe::egui::Pos2,
        snap: bool,
        config: &arkpets::PetConfig,
        now: Instant,
    ) -> (eframe::egui::Pos2, bool) {
        let value = [target.x, target.y];
        let transition = self
            .position
            .get_or_insert_with(|| Transition::new(value, now));
        if snap {
            transition.reset(value, now);
        }
        let ([x, y], animating) = transition.update(
            value,
            config.property_transition_duration,
            config.easing,
            now,
        );
        (eframe::egui::pos2(x, y), animating)
    }

    pub fn reset_position(&mut self, position: eframe::egui::Pos2, now: Instant) {
        self.position = Some(Transition::new([position.x, position.y], now));
    }

    pub fn rescale_position(&mut self, factor: f32) {
        if let Some(position) = &mut self.position {
            position.rescale(factor);
        }
    }

    pub fn facing(
        &mut self,
        direction: arkpets::Direction,
        config: &arkpets::PetConfig,
        now: Instant,
    ) -> (f32, bool) {
        let target = if direction == arkpets::Direction::Left {
            -1.0
        } else {
            1.0
        };
        let transition = self
            .facing
            .get_or_insert_with(|| Transition::new([target], now));
        let ([facing], animating) = transition.update(
            [target],
            config.property_transition_duration,
            config.easing,
            now,
        );
        (facing, animating)
    }
}

/// Visual property interpolation, independent of the paused simulation clock.
pub struct Transition<const N: usize> {
    from: [f32; N],
    target: [f32; N],
    started: Instant,
    duration: f32,
    curve: EasingFunction,
}

impl<const N: usize> Transition<N> {
    pub fn new(value: [f32; N], now: Instant) -> Self {
        assert!(value.iter().all(|value| value.is_finite()));
        Self {
            from: value,
            target: value,
            started: now,
            duration: 0.0,
            curve: EasingFunction::default(),
        }
    }

    pub fn update(
        &mut self,
        target: [f32; N],
        duration: f32,
        curve: EasingFunction,
        now: Instant,
    ) -> ([f32; N], bool) {
        assert!(target.iter().all(|value| value.is_finite()));
        assert!(duration.is_finite() && duration >= 0.0);
        if self.target != target || self.duration != duration || self.curve != curve {
            // Reversals and live settings edits start from the currently displayed value.
            self.from = self.sample(now).0;
            self.target = target;
            self.started = now;
            self.duration = duration;
            self.curve = curve;
        }
        self.sample(now)
    }

    pub fn sample(&self, now: Instant) -> ([f32; N], bool) {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();
        if self.from == self.target || elapsed >= self.duration {
            return (self.target, false);
        }
        let progress = elapsed / self.duration;
        let eased = match self.curve {
            EasingFunction::Linear => progress,
            EasingFunction::EaseOutSine => (progress * std::f32::consts::FRAC_PI_2).sin(),
            EasingFunction::EaseOutCubic => 1.0 - (1.0 - progress).powi(3),
            EasingFunction::EaseOutQuint => 1.0 - (1.0 - progress).powi(5),
        };
        (
            std::array::from_fn(|index| {
                self.from[index] * (1.0 - eased) + self.target[index] * eased
            }),
            true,
        )
    }

    pub fn reset(&mut self, value: [f32; N], now: Instant) {
        assert!(value.iter().all(|value| value.is_finite()));
        self.from = value;
        self.target = value;
        self.started = now;
    }

    fn rescale(&mut self, factor: f32) {
        assert!(factor.is_finite() && factor > 0.0);
        self.from = self.from.map(|value| value * factor);
        self.target = self.target.map(|value| value * factor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn both_position_axes_share_the_same_progress() {
        let start = Instant::now();
        let mut transition = Transition::new([10.0, 80.0], start);
        assert_eq!(
            transition.update([30.0, 40.0], 1.0, EasingFunction::Linear, start),
            ([10.0, 80.0], true)
        );
        assert_eq!(
            transition.sample(start + Duration::from_millis(250)),
            ([15.0, 70.0], true)
        );
        assert_eq!(
            transition.sample(start + Duration::from_secs(1)),
            ([30.0, 40.0], false)
        );
    }

    #[test]
    fn direction_flip_passes_through_zero() {
        let start = Instant::now();
        let mut transition = Transition::new([1.0], start);
        transition.update([-1.0], 1.0, EasingFunction::Linear, start);
        assert_eq!(
            transition.sample(start + Duration::from_millis(500)),
            ([0.0], true)
        );
        assert_eq!(
            transition.sample(start + Duration::from_secs(1)),
            ([-1.0], false)
        );
    }

    #[test]
    fn dpi_changes_preserve_the_physical_point_and_transition_progress() {
        let start = Instant::now();
        let mut transition = Transition::new([400.0, 200.0], start);
        transition.update([1000.0, 600.0], 1.0, EasingFunction::Linear, start);
        let middle = start + Duration::from_millis(500);
        let before = transition.sample(middle).0;
        transition.rescale(0.5);
        let (after, animating) = transition.sample(middle);
        assert!(animating);
        assert_eq!(after.map(|value| value * 2.0), before);
        assert_eq!(
            transition.sample(start + Duration::from_secs(1)),
            ([500.0, 300.0], false)
        );
    }

    #[test]
    fn original_curves_do_not_restart_for_the_same_target() {
        for (curve, midpoint) in [
            (EasingFunction::Linear, 0.5),
            (EasingFunction::EaseOutSine, std::f32::consts::FRAC_1_SQRT_2),
            (EasingFunction::EaseOutCubic, 0.875),
            (EasingFunction::EaseOutQuint, 0.96875),
        ] {
            let start = Instant::now();
            let mut transition = Transition::new([0.0], start);
            transition.update([1.0], 1.0, curve, start);
            for millis in [100, 200, 300, 400] {
                transition.update([1.0], 1.0, curve, start + Duration::from_millis(millis));
            }
            let (value, animating) =
                transition.update([1.0], 1.0, curve, start + Duration::from_millis(500));
            assert!(animating && (value[0] - midpoint).abs() < 0.00001);
            assert_eq!(
                transition.update([1.0], 1.0, curve, start + Duration::from_secs(1)),
                ([1.0], false)
            );
        }
    }

    #[test]
    fn reversing_a_transition_keeps_the_displayed_value_continuous() {
        let start = Instant::now();
        let mut transition = Transition::new([0.0, 100.0], start);
        transition.update([100.0, 0.0], 1.0, EasingFunction::Linear, start);
        let reverse_at = start + Duration::from_millis(250);
        assert_eq!(
            transition.update([0.0, 100.0], 1.0, EasingFunction::Linear, reverse_at),
            ([25.0, 75.0], true)
        );
        assert_eq!(
            transition.sample(reverse_at + Duration::from_millis(500)),
            ([12.5, 87.5], true)
        );
    }

    #[test]
    fn duration_and_curve_edits_restart_from_the_displayed_value() {
        let start = Instant::now();
        let mut transition = Transition::new([0.0], start);
        transition.update([1.0], 1.0, EasingFunction::Linear, start);
        let edit_at = start + Duration::from_millis(500);
        assert_eq!(
            transition.update([1.0], 2.0, EasingFunction::Linear, edit_at),
            ([0.5], true)
        );
        let edit_at = edit_at + Duration::from_secs(1);
        assert_eq!(
            transition.update([1.0], 2.0, EasingFunction::EaseOutCubic, edit_at),
            ([0.75], true)
        );
        assert_eq!(
            transition.sample(edit_at + Duration::from_secs(1)),
            ([0.96875], true)
        );
    }

    #[test]
    fn zero_duration_finishes_immediately_even_during_a_transition() {
        let start = Instant::now();
        let mut transition = Transition::new([0.0], start);
        transition.update([1.0], 1.0, EasingFunction::Linear, start);
        let now = start + Duration::from_millis(250);
        assert_eq!(
            transition.update([1.0], 0.0, EasingFunction::EaseOutCubic, now),
            ([1.0], false)
        );
        assert_eq!(
            transition.update([-1.0], 0.0, EasingFunction::EaseOutCubic, now),
            ([-1.0], false)
        );
    }

    #[test]
    fn reset_discards_the_previous_transition() {
        let start = Instant::now();
        let mut transition = Transition::new([0.0], start);
        transition.update([1.0], 1.0, EasingFunction::Linear, start);
        let now = start + Duration::from_millis(500);
        transition.reset([0.25], now);
        assert_eq!(transition.sample(now), ([0.25], false));
        assert_eq!(
            transition.sample(now + Duration::from_secs(2)),
            ([0.25], false)
        );
        assert_eq!(
            transition.update([0.75], 1.0, EasingFunction::Linear, now),
            ([0.25], true)
        );
    }

    #[test]
    #[should_panic]
    fn non_finite_initial_value_is_rejected() {
        Transition::new([f32::NAN], Instant::now());
    }

    #[test]
    #[should_panic]
    fn non_finite_target_is_rejected() {
        let now = Instant::now();
        Transition::new([0.0], now).update([f32::INFINITY], 1.0, EasingFunction::Linear, now);
    }

    #[test]
    #[should_panic]
    fn negative_duration_is_rejected() {
        let now = Instant::now();
        Transition::new([0.0], now).update([1.0], -1.0, EasingFunction::Linear, now);
    }
}
