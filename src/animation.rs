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

/// Markov selector that only chooses animations present in the loaded skeleton.
pub struct AnimationMachine {
    available: Vec<String>,
    enabled: Vec<String>,
    pub state: AnimationState,
    rng: StdRng,
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
        let initial = if available.iter().any(|name| name == "Relax") {
            "Relax".to_string()
        } else {
            available[0].clone()
        };
        Self {
            enabled: available.clone(),
            available,
            state: AnimationState {
                animation: initial,
                ..AnimationState::default()
            },
            rng: StdRng::seed_from_u64(seed),
        }
    }

    pub fn available(&self) -> &[String] {
        &self.available
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
        self.enabled = self
            .available
            .iter()
            .filter(|name| match name.as_str() {
                "Sit" => config.allow_sit,
                "Sleep" => config.allow_sleep,
                "Move" => config.allow_walk,
                "Special" => config.allow_special,
                _ => true,
            })
            .cloned()
            .collect();
    }

    pub fn tick(&mut self, dt: f32) {
        if dt.is_finite() {
            self.state.timestamp += dt.clamp(0.0, 0.1);
        }
    }

    pub fn complete(&mut self) {
        let current = self.state.animation.clone();
        let names = self.transition_names();
        let row = transition_row(&names, &current);
        let next = pick(&names, row, &mut self.rng);
        if current == "Relax" && next == "Move" && self.rng.gen_bool(0.4) {
            self.state.direction = match self.state.direction {
                Direction::Left => Direction::Right,
                Direction::Right => Direction::Left,
            };
        }
        self.state.animation = next;
        self.state.timestamp = 0.0;
    }

    pub fn set_direction(&mut self, direction: Direction) {
        self.state.direction = direction;
    }

    fn transition_names(&self) -> Vec<String> {
        let has = |name: &str| self.enabled.iter().any(|item| item == name);
        let mut names = vec!["Relax".to_string(), "Interact".to_string()];
        if has("Move") {
            names.push("Move".into());
        }
        if has("Sit") {
            names.push("Sit".into());
        }
        if has("Sleep") {
            names.push("Sleep".into());
        }
        if has("Special") {
            names.push("Special".into());
        }
        names.retain(|name| has(name));
        if names.is_empty() {
            names.push(self.state.animation.clone());
        }
        names
    }
}

fn transition_row(names: &[String], current: &str) -> Vec<u32> {
    // Web implementation's rows are intentionally preserved for compatible
    // models. Missing states are removed and their probability is redistributed.
    let normal = [
        ("Relax", [50, 0, 25, 15, 10]),
        ("Interact", [100, 0, 0, 0, 0]),
        ("Move", [20, 0, 80, 0, 0]),
        ("Sit", [30, 0, 0, 70, 0]),
        ("Sleep", [10, 0, 0, 0, 90]),
    ];
    let vehicle = [
        ("Relax", [50, 0, 50]),
        ("Interact", [100, 0, 0]),
        ("Move", [30, 0, 70]),
    ];
    let source = if names
        .iter()
        .all(|name| matches!(name.as_str(), "Relax" | "Interact" | "Move"))
    {
        vehicle
            .iter()
            .find(|(name, _)| *name == current)
            .map(|(_, row)| row.to_vec())
    } else {
        normal
            .iter()
            .find(|(name, _)| *name == current)
            .map(|(_, row)| row.to_vec())
    }
    .unwrap_or_else(|| vec![1; names.len()]);
    names
        .iter()
        .map(|name| match name.as_str() {
            "Relax" => source.first().copied().unwrap_or(0),
            "Interact" => source.get(1).copied().unwrap_or(0),
            "Move" => source.get(2).copied().unwrap_or(0),
            "Sit" => source.get(3).copied().unwrap_or(0),
            "Sleep" => source.get(4).copied().unwrap_or(0),
            "Special" if current == "Relax" => 10,
            _ => 0,
        })
        .collect()
}

fn pick(names: &[String], weights: Vec<u32>, rng: &mut StdRng) -> String {
    let total: u32 = weights.iter().sum();
    if total == 0 {
        return names[0].clone();
    }
    let mut target = rng.gen_range(0..total);
    for (name, weight) in names.iter().zip(weights) {
        if target < weight {
            return name.clone();
        }
        target -= weight;
    }
    names[0].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn disabled_actions_never_appear_in_automatic_transitions() {
        let mut machine = AnimationMachine::with_seed(
            ["Relax", "Move", "Sit", "Sleep", "Special"]
                .into_iter()
                .map(str::to_owned),
            7,
        );
        machine.configure(&crate::PetConfig {
            allow_walk: false,
            allow_sit: false,
            allow_sleep: false,
            allow_special: false,
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
