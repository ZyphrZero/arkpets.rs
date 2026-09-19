use super::{renderer::Effects, transition::Transition};
use arkpets::{config::OutlineCondition, PetConfig};
use std::time::Instant;

#[derive(Clone, Copy, Default)]
pub struct Input {
    pub dragging: bool,
    pub pressing: bool,
    pub focused: bool,
    pub controlled: bool,
}

#[derive(Default)]
pub struct EffectState {
    outline: Option<Transition<1>>,
}

impl EffectState {
    pub fn update(
        &mut self,
        config: &PetConfig,
        input: Input,
        opacity: f32,
        pixels_per_point: f32,
        now: Instant,
    ) -> (Effects, bool) {
        let outline = &config.rendering.outline;
        let condition = if input.controlled {
            outline.controlled_condition
        } else {
            outline.condition
        };
        let enabled = match condition {
            OutlineCondition::Never => false,
            OutlineCondition::Dragging => input.dragging,
            OutlineCondition::Pressing => input.pressing,
            OutlineCondition::Focused => input.focused,
            OutlineCondition::Always => true,
        } && outline.width > 0.0;
        let transition = self
            .outline
            .get_or_insert_with(|| Transition::new([0.0], now));
        let ([alpha], animating) = transition.update(
            [if enabled { 1.0 } else { 0.0 }],
            config.property_transition_duration,
            config.easing,
            now,
        );
        let mut color = if input.controlled {
            outline.controlled_color
        } else {
            outline.color
        }
        .map(|v| v as f32 / 255.0);
        color[3] *= alpha;
        (
            Effects {
                opacity,
                outline_color: color,
                outline_width: outline.width * pixels_per_point,
                shadow_opacity: config.rendering.shadow_opacity,
                high_quality: config.rendering.high_quality,
                mipmaps: config.rendering.mipmaps,
            },
            animating,
        )
    }
}

/// Room for either quality's complete kernel, including future condition changes.
pub fn padding(config: &PetConfig) -> f32 {
    (config.rendering.outline.width * 2.0 + 2.0).max(20.0 * config.display_scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn conditions_and_manual_mode_use_their_own_color_without_changing_opacity() {
        let mut config = PetConfig {
            property_transition_duration: 0.0,
            ..Default::default()
        };
        config.rendering.outline.controlled_condition = OutlineCondition::Focused;
        let now = Instant::now();
        for (condition, input, enabled) in [
            (
                OutlineCondition::Never,
                Input {
                    dragging: true,
                    pressing: true,
                    focused: true,
                    ..Default::default()
                },
                false,
            ),
            (
                OutlineCondition::Dragging,
                Input {
                    dragging: true,
                    ..Default::default()
                },
                true,
            ),
            (
                OutlineCondition::Dragging,
                Input {
                    pressing: true,
                    ..Default::default()
                },
                false,
            ),
            (
                OutlineCondition::Pressing,
                Input {
                    pressing: true,
                    ..Default::default()
                },
                true,
            ),
            (OutlineCondition::Focused, Input::default(), false),
            (
                OutlineCondition::Focused,
                Input {
                    focused: true,
                    ..Default::default()
                },
                true,
            ),
            (OutlineCondition::Always, Input::default(), true),
        ] {
            config.rendering.outline.condition = condition;
            let (effects, active) = EffectState::default().update(&config, input, 0.25, 2.0, now);
            assert_eq!(effects.outline_color[3] > 0.0, enabled);
            assert!(!active);
            assert_eq!(effects.opacity, 0.25);
            assert_eq!(effects.outline_width, 4.0);
        }
        let mut state = EffectState::default();
        let (effects, _) = state.update(
            &config,
            Input {
                controlled: true,
                focused: true,
                ..Default::default()
            },
            1.0,
            1.0,
            now,
        );
        assert_eq!(effects.outline_color, [1.0, 187.0 / 255.0, 0.0, 1.0]);
        let (effects, _) = state.update(
            &config,
            Input {
                controlled: true,
                dragging: true,
                ..Default::default()
            },
            1.0,
            1.0,
            now,
        );
        assert_eq!(
            effects.outline_color[3], 0.0,
            "manual mode is not the same as dragging"
        );
    }

    #[test]
    fn outline_fades_with_common_properties_and_reaches_idle() {
        let config = PetConfig::default();
        let now = Instant::now();
        let mut state = EffectState::default();
        let dragging = Input {
            dragging: true,
            ..Default::default()
        };
        assert!(state.update(&config, dragging, 1.0, 1.0, now).1);
        let (effects, active) = state.update(
            &config,
            dragging,
            1.0,
            1.0,
            now + Duration::from_millis(150),
        );
        assert!(active && effects.outline_color[3] > 0.0 && effects.outline_color[3] < 1.0);
        assert!(
            !state
                .update(&config, dragging, 1.0, 1.0, now + Duration::from_secs(1))
                .1
        );
        assert!(
            state
                .update(
                    &config,
                    Input::default(),
                    1.0,
                    1.0,
                    now + Duration::from_secs(1)
                )
                .1
        );
        let (effects, active) = state.update(
            &config,
            Input::default(),
            1.0,
            1.0,
            now + Duration::from_secs(2),
        );
        assert!(!active && effects.outline_color[3] == 0.0);
    }
}
