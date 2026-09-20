use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Left,
    #[default]
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationState {
    pub animation: String,
    pub direction: Direction,
    pub timestamp: f32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            animation: "Relax".into(),
            direction: Direction::Right,
            timestamp: 0.0,
        }
    }
}

pub fn animation_loops(name: &str) -> bool {
    // Only original base actions have built-in playback semantics. Imported
    // clip names, including Begin/End/Loop, are otherwise opaque.
    !matches!(name, "Interact" | "Special")
}

pub fn initial_animation(available: &[String]) -> Option<&str> {
    ["Relax", "Idle", "Stand"]
        .into_iter()
        .find(|name| available.iter().any(|item| item == name))
        .or_else(|| available.first().map(String::as_str))
}

pub fn movement_animation(available: &[String]) -> Option<&str> {
    available
        .iter()
        .find(|name| name.as_str() == "Move")
        .map(String::as_str)
}

pub fn manual_animations(available: &[String]) -> Vec<&str> {
    available
        .iter()
        .map(String::as_str)
        .filter(|name| *name != "Move")
        .collect()
}

pub fn animation_allowed(name: &str, config: &crate::PetConfig) -> bool {
    match name {
        "Sit" => config.allow_sit,
        "Sleep" => config.allow_sleep,
        "Move" => config.allow_walk,
        "Special" => config.allow_special,
        _ => true,
    }
}

/// Markov selector that only chooses animations present in the loaded skeleton.
pub struct AnimationMachine {
    available: Vec<String>,
    enabled: Vec<String>,
    pub state: AnimationState,
    rng: StdRng,
    activity_level: u8,
}

impl AnimationMachine {
    pub fn new(available: impl IntoIterator<Item = String>) -> Self {
        Self::with_seed(available, rand::random())
    }

    pub fn with_seed(available: impl IntoIterator<Item = String>, seed: u64) -> Self {
        let mut available: Vec<String> = available.into_iter().collect();
        if available.is_empty() {
            available = vec!["Relax", "Interact", "Move", "Sit", "Sleep"]
                .into_iter()
                .map(String::from)
                .collect();
        }
        let initial = initial_animation(&available).unwrap().to_owned();
        Self {
            enabled: available.clone(),
            available,
            state: AnimationState {
                animation: initial,
                ..AnimationState::default()
            },
            rng: StdRng::seed_from_u64(seed),
            activity_level: crate::PetConfig::default().activity_level,
        }
    }

    pub fn available(&self) -> &[String] {
        &self.available
    }

    pub fn set_available(&mut self, available: Vec<String>, config: &crate::PetConfig) {
        assert!(!available.is_empty(), "a form must provide an animation");
        self.available = available;
        self.state.animation = initial_animation(&self.available).unwrap().to_owned();
        self.state.timestamp = 0.0;
        self.configure(config);
    }

    pub fn play(&mut self, name: impl Into<String>) -> bool {
        let name = name.into();
        if self.available.iter().any(|item| item == &name) {
            self.state.animation = name;
            self.state.timestamp = 0.0;
            true
        } else {
            false
        }
    }

    pub fn interact(&mut self) -> bool {
        self.play("Interact")
    }

    pub fn configure(&mut self, config: &crate::PetConfig) {
        self.activity_level = config.activity_level.min(16);
        self.enabled = self
            .available
            .iter()
            .filter(|name| animation_allowed(name, config))
            .cloned()
            .collect();
    }

    pub fn tick(&mut self, dt: f32) {
        if dt.is_finite() {
            self.state.timestamp += dt.clamp(0.0, 0.1);
        }
    }

    pub fn complete(&mut self) {
        // Without the base Relax action, keep the selected clip. Do not infer
        // combat states or transitions from names in an arbitrary skeleton.
        if !self.enabled.iter().any(|name| name == "Relax") {
            return;
        }
        let names = ["Relax", "Sit", "Sleep", "Move", "Special"];
        let mut weights = transition_weights(&self.state.animation);
        for (index, name) in names.iter().enumerate() {
            weights[index] *= if !self.enabled.iter().any(|item| item == name) {
                0
            } else if index == 0 {
                u32::from(16 - self.activity_level)
            } else {
                8
            };
        }
        let total: u32 = weights.iter().sum();
        let index = if total > 0 {
            let mut target = self.rng.gen_range(0..total);
            weights
                .iter()
                .position(|weight| {
                    if target < *weight {
                        true
                    } else {
                        target -= weight;
                        false
                    }
                })
                .unwrap()
        } else {
            0
        };
        self.state.animation = names[index].to_owned();
        if names[index] == "Move" {
            self.state.direction = if self.rng.gen_bool(0.5) {
                Direction::Left
            } else {
                Direction::Right
            };
        }
        self.state.timestamp = 0.0;
    }

    pub fn set_direction(&mut self, direction: Direction) {
        self.state.direction = direction;
    }
}

fn transition_weights(current: &str) -> [u32; 5] {
    // Java base weights, with symmetric left/right movement merged.
    //            Relax Sit Sleep Move Special
    match current {
        "Relax" => [40, 20, 10, 20, 10],
        "Sit" => [30, 40, 20, 20, 10],
        "Sleep" => [20, 20, 60, 0, 0],
        "Move" => [40, 10, 0, 40, 10],
        "Special" => [50, 20, 10, 20, 0],
        _ => [100, 0, 0, 0, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_base_clips_remain_manual_and_names_do_not_trigger_transitions() {
        let names = [
            "Idle",
            "Attack",
            "Combat2_loop",
            "Skill_1_Begin",
            "Skill_1_End",
            "Move_Loop",
            "Start",
        ];
        let mut machine = AnimationMachine::with_seed(names.map(str::to_owned), 42);
        assert_eq!(machine.state.animation, "Idle");
        for name in names {
            assert!(machine.play(name));
            for _ in 0..100 {
                machine.complete();
                assert_eq!(machine.state.animation, name);
            }
        }
        assert!(!machine.interact());
        assert_eq!(manual_animations(machine.available()), names);
        assert!(movement_animation(machine.available()).is_none());
    }

    #[test]
    fn base_actions_do_not_promote_similarly_named_clips() {
        let config = crate::PetConfig {
            allow_sit: false,
            allow_sleep: false,
            allow_walk: false,
            allow_special: false,
            ..Default::default()
        };
        let names = [
            "Relax",
            "Move2",
            "Sit_Loop",
            "Sleep_Begin",
            "Special_Loop",
            "Attack",
        ];
        let mut machine = AnimationMachine::with_seed(names.map(str::to_owned), 42);
        machine.configure(&config);
        for _ in 0..100 {
            machine.complete();
            assert_eq!(machine.state.animation, "Relax");
        }
        for name in &names[1..] {
            assert!(animation_allowed(name, &config));
            assert!(animation_loops(name));
        }
        for name in ["Move", "Sit", "Sleep", "Special"] {
            assert!(!animation_allowed(name, &config));
        }
        assert!(!animation_loops("Interact"));
    }

    #[test]
    fn activity_changes_idle_frequency_without_changing_the_animation_clock() {
        let mut counts = Vec::new();
        for activity in [0, 4, 8, 16] {
            let mut machine = AnimationMachine::with_seed(
                ["Relax", "Move", "Sit", "Sleep"].map(str::to_owned),
                42,
            );
            machine.configure(&crate::PetConfig {
                activity_level: activity,
                ..Default::default()
            });
            let mut idle = 0;
            for _ in 0..10_000 {
                machine.complete();
                idle += usize::from(machine.state.animation == "Relax");
            }
            machine.tick(0.05);
            assert_eq!(machine.state.timestamp, 0.05);
            counts.push(idle);
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] > pair[1]),
            "{counts:?}"
        );
        let mut idle_only = AnimationMachine::with_seed(["Relax".into()], 42);
        idle_only.configure(&crate::PetConfig {
            activity_level: 16,
            ..Default::default()
        });
        idle_only.complete();
        assert_eq!(idle_only.state.animation, "Relax");
    }

    #[test]
    fn vehicle_does_not_select_sit_or_sleep() {
        let mut machine = AnimationMachine::with_seed(
            ["Relax", "Interact", "Move"]
                .into_iter()
                .map(str::to_string),
            7,
        );
        for _ in 0..100 {
            machine.complete();
            assert!(matches!(
                machine.state.animation.as_str(),
                "Relax" | "Interact" | "Move"
            ));
        }
    }

    #[test]
    fn invalid_animation_is_rejected() {
        let mut machine = AnimationMachine::with_seed(["Relax".to_string()], 1);
        assert!(!machine.play("Sleep"));
    }

    #[test]
    fn base_automatic_actions_follow_java_idle_weights_and_both_walk_directions() {
        let mut machine = AnimationMachine::with_seed(
            ["Relax", "Sit", "Sleep", "Move", "Special", "Interact"].map(str::to_owned),
            42,
        );
        machine.configure(&crate::PetConfig {
            activity_level: 8,
            allow_sleep: true,
            ..Default::default()
        });
        let mut counts = [0_i32; 6];
        for _ in 0..10_000 {
            machine.play("Relax");
            machine.complete();
            let index = match machine.state.animation.as_str() {
                "Relax" => 0,
                "Sit" => 1,
                "Sleep" => 2,
                "Move" if machine.state.direction == Direction::Left => 3,
                "Move" => 4,
                "Special" => 5,
                unexpected => panic!("unexpected random clip: {unexpected}"),
            };
            counts[index] += 1;
        }
        for (actual, expected) in counts.into_iter().zip([4000, 2000, 1000, 1000, 1000, 1000]) {
            assert!((actual - expected).abs() < 200, "{counts:?}");
        }
    }

    #[test]
    fn disabled_actions_never_appear_in_automatic_transitions() {
        let mut machine = AnimationMachine::with_seed(
            ["Relax", "Move", "Sit", "Sleep", "Special", "Interact"]
                .into_iter()
                .map(str::to_owned),
            7,
        );
        machine.configure(&crate::PetConfig {
            allow_walk: false,
            allow_sit: false,
            allow_sleep: false,
            allow_special: false,
            allow_interact: false,
            ..Default::default()
        });
        for _ in 0..500 {
            machine.complete();
            assert_eq!(machine.state.animation, "Relax");
        }
    }

    #[test]
    fn custom_idle_is_not_replaced_with_an_unavailable_animation() {
        let mut machine = AnimationMachine::with_seed(["Idle".to_owned()], 2);
        for _ in 0..20 {
            machine.complete();
            assert_eq!(machine.state.animation, "Idle");
        }
    }
}
