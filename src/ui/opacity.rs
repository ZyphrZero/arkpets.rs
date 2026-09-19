use super::transition::Transition;
use arkpets::PetConfig;
use std::time::Instant;

/// A desktop-only visual transition, independent of the paused simulation clock.
pub struct OpacityFade {
    transition: Transition<1>,
}

impl OpacityFade {
    pub fn new(config: &PetConfig, transparent: bool, now: Instant) -> Self {
        let target = target_opacity(config, transparent);
        Self {
            transition: Transition::new([target], now),
        }
    }

    pub fn sample(&self, now: Instant) -> (f32, bool) {
        let ([opacity], animating) = self.transition.sample(now);
        (opacity, animating)
    }

    pub fn update(&mut self, config: &PetConfig, transparent: bool, now: Instant) -> (f32, bool) {
        self.transition.update(
            [target_opacity(config, transparent)],
            config.property_transition_duration,
            config.easing,
            now,
        );
        self.sample(now)
    }
}

fn target_opacity(config: &PetConfig, transparent: bool) -> f32 {
    if transparent {
        // Old preferences could store dim > normal; transparent mode must not brighten.
        config.opacity_dim.min(config.opacity_normal)
    } else {
        config.opacity_normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkpets::config::EasingFunction;
    use std::time::Duration;

    #[test]
    fn original_curves_reach_the_target_without_restarting_each_frame() {
        for (curve, midpoint) in [
            (EasingFunction::Linear, 0.5),
            (
                EasingFunction::EaseOutSine,
                1.0 - std::f32::consts::FRAC_1_SQRT_2,
            ),
            (EasingFunction::EaseOutCubic, 0.125),
            (EasingFunction::EaseOutQuint, 0.03125),
        ] {
            let config = PetConfig {
                opacity_dim: 0.0,
                property_transition_duration: 1.0,
                easing: curve,
                ..Default::default()
            };
            let start = Instant::now();
            let mut fade = OpacityFade::new(&config, false, start);
            assert_eq!(fade.update(&config, true, start), (1.0, true));
            for millis in [100, 200, 300, 400] {
                fade.update(&config, true, start + Duration::from_millis(millis));
            }
            let (alpha, animating) = fade.update(&config, true, start + Duration::from_millis(500));
            assert!(animating && (alpha - midpoint).abs() < 0.00001);
            assert_eq!(
                fade.update(&config, true, start + Duration::from_secs(1)),
                (0.0, false)
            );
        }
    }

    #[test]
    fn rapid_toggle_and_live_settings_continue_from_the_current_opacity() {
        let mut config = PetConfig {
            opacity_dim: 0.2,
            property_transition_duration: 1.0,
            easing: EasingFunction::Linear,
            ..Default::default()
        };
        let start = Instant::now();
        let mut fade = OpacityFade::new(&config, false, start);
        fade.update(&config, true, start);
        let now = start + Duration::from_millis(500);
        assert!((fade.update(&config, false, now).0 - 0.6).abs() < 0.00001);
        config.opacity_normal = 0.8;
        config.easing = EasingFunction::EaseOutQuint;
        assert!((fade.update(&config, false, now).0 - 0.6).abs() < 0.00001);
        config.property_transition_duration = 0.0;
        assert_eq!(fade.update(&config, false, now), (0.8, false));
        assert_eq!(fade.update(&config, true, now), (0.2, false));
    }

    #[test]
    fn opening_uses_the_selected_mode_and_dim_cannot_brighten_the_pet() {
        let config = PetConfig {
            opacity_normal: 0.4,
            ..Default::default()
        };
        let now = Instant::now();
        let mut fade = OpacityFade::new(&config, true, now);
        assert_eq!(fade.sample(now), (0.4, false));
        assert_eq!(fade.update(&config, false, now), (0.4, false));
    }
}
