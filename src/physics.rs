use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn max_position(self, size: Vec2) -> Vec2 {
        Vec2::new(
            (self.width - size.x).max(0.0),
            (self.height - size.y).max(0.0),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "PhysicsConfigInput")]
pub struct PhysicsConfig {
    pub gravity: f32,
    pub air_friction: f32,
    pub ground_friction: f32,
    /// Legacy per-frame damping, retained for previously saved configurations.
    pub drag: f32,
    pub max_velocity: Vec2,
    pub min_velocity: f32,
    pub bounce_damping: f32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            gravity: 800.0,
            air_friction: 100.0,
            ground_friction: 500.0,
            drag: 1.0,
            max_velocity: Vec2::new(1000.0, 1000.0),
            min_velocity: 0.0,
            bounce_damping: 0.7,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct PhysicsConfigInput {
    #[serde(deserialize_with = "deserialize_present")]
    gravity: Option<f32>,
    #[serde(deserialize_with = "deserialize_present")]
    air_friction: Option<f32>,
    #[serde(deserialize_with = "deserialize_present")]
    ground_friction: Option<f32>,
    #[serde(deserialize_with = "deserialize_present")]
    drag: Option<f32>,
    #[serde(deserialize_with = "deserialize_present")]
    max_velocity: Option<Vec2>,
    #[serde(deserialize_with = "deserialize_present")]
    min_velocity: Option<f32>,
    #[serde(deserialize_with = "deserialize_present")]
    bounce_damping: Option<f32>,
}

// Option tracks field presence internally; the persisted fields are bare values
// in JSON and RON, rather than RON's explicit Some(...) representation.
fn deserialize_present<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl From<PhysicsConfigInput> for PhysicsConfig {
    fn from(input: PhysicsConfigInput) -> Self {
        // Old preferences used exponential damping instead of acceleration.
        // Preserve that behavior until a user opts into the new friction values.
        let defaults = if input.drag.is_some()
            && input.air_friction.is_none()
            && input.ground_friction.is_none()
        {
            Self {
                gravity: 1000.0,
                air_friction: 0.0,
                ground_friction: 0.0,
                drag: 0.98,
                min_velocity: 5.0,
                ..Self::default()
            }
        } else {
            Self::default()
        };
        Self {
            gravity: input.gravity.unwrap_or(defaults.gravity),
            air_friction: input.air_friction.unwrap_or(defaults.air_friction),
            ground_friction: input.ground_friction.unwrap_or(defaults.ground_friction),
            drag: input.drag.unwrap_or(defaults.drag),
            max_velocity: input.max_velocity.unwrap_or(defaults.max_velocity),
            min_velocity: input.min_velocity.unwrap_or(defaults.min_velocity),
            bounce_damping: input.bounce_damping.unwrap_or(defaults.bounce_damping),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsState {
    pub position: Vec2,
    pub velocity: Vec2,
    pub dragging: bool,
    #[serde(skip)]
    drag_anchor: Vec2,
}

impl Default for PhysicsState {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            velocity: Vec2::ZERO,
            dragging: false,
            drag_anchor: Vec2::ZERO,
        }
    }
}

impl PhysicsState {
    pub fn translate(&mut self, delta: Vec2) {
        self.position.x += delta.x;
        self.position.y += delta.y;
        if self.dragging {
            self.drag_anchor.x -= delta.x;
            self.drag_anchor.y -= delta.y;
        }
    }

    pub fn start_drag(&mut self, cursor: Vec2) {
        self.dragging = true;
        self.velocity = Vec2::ZERO;
        self.drag_anchor = Vec2::new(cursor.x - self.position.x, cursor.y - self.position.y);
    }

    pub fn drag_to(
        &mut self,
        cursor: Vec2,
        dt: f32,
        bounds: Bounds,
        size: Vec2,
        max_velocity: Vec2,
    ) {
        if !self.dragging {
            return;
        }
        let old = self.position;
        let next = Vec2::new(cursor.x - self.drag_anchor.x, cursor.y - self.drag_anchor.y);
        self.position = clamp_position(next, bounds.max_position(size));
        if dt.is_finite() && dt > 0.0 {
            self.velocity = Vec2::new(
                ((self.position.x - old.x) / dt).clamp(-max_velocity.x, max_velocity.x),
                ((self.position.y - old.y) / dt).clamp(-max_velocity.y, max_velocity.y),
            );
        } else {
            self.velocity = Vec2::ZERO;
        }
    }

    pub fn end_drag(&mut self) {
        self.dragging = false;
        self.drag_anchor = Vec2::ZERO;
    }

    /// Advances gravity, friction, wall/floor collision, and optional walking.
    pub fn step(
        &mut self,
        dt: f32,
        bounds: Bounds,
        size: Vec2,
        config: PhysicsConfig,
        walking: Option<(crate::Direction, f32)>,
    ) {
        let grounded = self.position.y >= bounds.max_position(size).y - 0.001;
        self.step_with_ground_contact(dt, bounds, size, config, walking, grounded);
    }

    /// Advances physics using a caller-provided ground contact, for surfaces
    /// such as a monitor's floor within a larger desktop bounding rectangle.
    pub fn step_with_ground_contact(
        &mut self,
        dt: f32,
        bounds: Bounds,
        size: Vec2,
        config: PhysicsConfig,
        walking: Option<(crate::Direction, f32)>,
        grounded: bool,
    ) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.1);
        let max = bounds.max_position(size);

        if !self.dragging {
            let grounded = grounded && self.velocity.y >= 0.0;
            self.velocity.y += config.gravity * dt;
            if grounded {
                self.velocity.x = apply_friction(self.velocity.x, config.ground_friction * dt);
            }
            self.velocity.x = apply_friction(self.velocity.x, config.air_friction * dt);
            self.velocity.y = apply_friction(self.velocity.y, config.air_friction * dt);
            let damping = config.drag.powf(dt * 60.0);
            self.velocity.x *= damping;
            self.velocity.y *= damping;
            if self.velocity.x.abs() < config.min_velocity {
                self.velocity.x = 0.0;
            }
            if self.velocity.y.abs() < config.min_velocity {
                self.velocity.y = 0.0;
            }
            self.velocity.x = self
                .velocity
                .x
                .clamp(-config.max_velocity.x, config.max_velocity.x);
            self.velocity.y = self
                .velocity
                .y
                .clamp(-config.max_velocity.y, config.max_velocity.y);
            self.position.x += self.velocity.x * dt;
            self.position.y += self.velocity.y * dt;

            if self.position.x < 0.0 {
                self.position.x = 0.0;
                self.velocity.x = -self.velocity.x * config.bounce_damping;
            } else if self.position.x > max.x {
                self.position.x = max.x;
                self.velocity.x = -self.velocity.x * config.bounce_damping;
            }
            if self.position.y < 0.0 {
                self.position.y = 0.0;
                self.velocity.y = 0.0;
            } else if self.position.y > max.y {
                self.position.y = max.y;
                self.velocity.y = 0.0;
            }
        }

        if let Some((direction, speed)) = walking.filter(|_| !self.dragging) {
            self.position.x += match direction {
                crate::Direction::Left => -speed * dt,
                crate::Direction::Right => speed * dt,
            };
            if self.position.x <= 0.0 {
                self.position.x = 0.0;
            } else if self.position.x >= max.x {
                self.position.x = max.x;
            }
        }
        self.position = clamp_position(self.position, max);
    }
}

fn apply_friction(velocity: f32, amount: f32) -> f32 {
    velocity.signum() * (velocity.abs() - amount).max(0.0)
}

fn clamp_position(position: Vec2, max: Vec2) -> Vec2 {
    Vec2::new(position.x.clamp(0.0, max.x), position.y.clamp(0.0, max.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_stops_at_floor() {
        let mut state = PhysicsState::default();
        for _ in 0..100 {
            state.step(
                1.0 / 60.0,
                Bounds::new(300.0, 200.0),
                Vec2::new(50.0, 50.0),
                PhysicsConfig::default(),
                None,
            );
        }
        assert_eq!(state.position.y, 150.0);
        assert_eq!(state.velocity.y, 0.0);
    }

    #[test]
    fn drag_release_preserves_velocity() {
        let mut state = PhysicsState::default();
        state.start_drag(Vec2::new(10.0, 10.0));
        state.drag_to(
            Vec2::new(40.0, 50.0),
            0.1,
            Bounds::new(200.0, 200.0),
            Vec2::new(20.0, 20.0),
            PhysicsConfig::default().max_velocity,
        );
        state.end_drag();
        assert_eq!(state.velocity, Vec2::new(300.0, 400.0));
    }

    #[test]
    fn tiny_drag_intervals_keep_release_velocity_finite_and_bounded() {
        let mut state = PhysicsState::default();
        let bounds = Bounds::new(200.0, 200.0);
        let size = Vec2::new(20.0, 20.0);
        let limit = Vec2::new(500.0, 250.0);
        state.start_drag(Vec2::new(10.0, 10.0));
        state.drag_to(
            Vec2::new(1000.0, 1000.0),
            f32::MIN_POSITIVE,
            bounds,
            size,
            limit,
        );
        assert_eq!(state.position, Vec2::new(180.0, 180.0));
        assert_eq!(state.velocity, limit);
        state.end_drag();
        assert!(state.velocity.x.is_finite() && state.velocity.y.is_finite());
        state.start_drag(Vec2::new(190.0, 190.0));
        state.drag_to(Vec2::ZERO, f32::MIN_POSITIVE, bounds, size, limit);
        assert_eq!(state.position, Vec2::ZERO);
        assert_eq!(state.velocity, Vec2::new(-limit.x, -limit.y));
    }

    #[test]
    fn invalid_drag_interval_does_not_keep_previous_throw_velocity() {
        let bounds = Bounds::new(200.0, 200.0);
        let size = Vec2::new(20.0, 20.0);
        let limit = PhysicsConfig::default().max_velocity;
        for dt in [0.0, -0.01, f32::NAN, f32::INFINITY] {
            let mut state = PhysicsState::default();
            state.start_drag(Vec2::ZERO);
            state.drag_to(Vec2::new(20.0, 20.0), 0.1, bounds, size, limit);
            assert_ne!(state.velocity, Vec2::ZERO);
            state.drag_to(Vec2::new(30.0, 30.0), dt, bounds, size, limit);
            state.end_drag();
            assert_eq!(state.position, Vec2::new(30.0, 30.0));
            assert_eq!(state.velocity, Vec2::ZERO);
        }
    }

    fn friction_config(air_friction: f32, ground_friction: f32) -> PhysicsConfig {
        PhysicsConfig {
            gravity: 0.0,
            air_friction,
            ground_friction,
            ..PhysicsConfig::default()
        }
    }

    fn airborne_state(velocity: Vec2) -> PhysicsState {
        PhysicsState {
            position: Vec2::new(1000.0, 1000.0),
            velocity,
            ..PhysicsState::default()
        }
    }

    #[test]
    fn air_friction_slows_both_axes_without_ground_friction() {
        let mut state = airborne_state(Vec2::new(100.0, -100.0));
        state.step(
            0.1,
            Bounds::new(10_000.0, 10_000.0),
            Vec2::ZERO,
            friction_config(100.0, 500.0),
            None,
        );
        assert_eq!(state.velocity, Vec2::new(90.0, -90.0));
    }

    #[test]
    fn gravity_is_applied_before_air_friction() {
        let mut state = airborne_state(Vec2::ZERO);
        state.step(
            0.1,
            Bounds::new(10_000.0, 10_000.0),
            Vec2::ZERO,
            PhysicsConfig::default(),
            None,
        );
        assert_eq!(state.velocity.y, 70.0);
    }

    #[test]
    fn ground_friction_adds_to_air_friction_only_on_horizontal_axis() {
        let mut state = airborne_state(Vec2::new(100.0, 100.0));
        state.step_with_ground_contact(
            0.1,
            Bounds::new(10_000.0, 10_000.0),
            Vec2::ZERO,
            friction_config(100.0, 500.0),
            None,
            true,
        );
        assert_eq!(state.velocity, Vec2::new(40.0, 90.0));

        state.position.y = 10_000.0;
        state.velocity = Vec2::new(-100.0, 0.0);
        state.step(
            0.1,
            Bounds::new(10_000.0, 10_000.0),
            Vec2::ZERO,
            friction_config(100.0, 500.0),
            None,
        );
        assert_eq!(state.velocity, Vec2::new(-40.0, 0.0));
    }

    #[test]
    fn upward_throw_does_not_receive_ground_friction() {
        let mut state = airborne_state(Vec2::new(100.0, -100.0));
        state.position.y = 10_000.0;
        state.step(
            0.1,
            Bounds::new(10_000.0, 10_000.0),
            Vec2::ZERO,
            PhysicsConfig::default(),
            None,
        );
        assert_eq!(state.velocity, Vec2::new(90.0, -10.0));
        assert!(state.position.y < 10_000.0);
    }

    #[test]
    fn friction_stops_small_velocities_without_reversing_them() {
        for velocity in [-10.0, -5.0, 0.0, 5.0, 10.0] {
            let mut state = airborne_state(Vec2::new(velocity, velocity));
            state.step(
                0.1,
                Bounds::new(10_000.0, 10_000.0),
                Vec2::ZERO,
                friction_config(100.0, 500.0),
                None,
            );
            assert_eq!(state.velocity, Vec2::ZERO);
        }
    }

    #[test]
    fn zero_friction_preserves_velocity() {
        let mut state = airborne_state(Vec2::new(-100.0, 100.0));
        state.step_with_ground_contact(
            0.1,
            Bounds::new(10_000.0, 10_000.0),
            Vec2::ZERO,
            friction_config(0.0, 0.0),
            None,
            true,
        );
        assert_eq!(state.velocity, Vec2::new(-100.0, 100.0));
    }

    #[test]
    fn friction_acceleration_has_consistent_velocity_across_frame_rates() {
        for fps in [30, 60, 120] {
            let mut airborne = airborne_state(Vec2::new(800.0, 100.0));
            let mut grounded = airborne_state(Vec2::new(800.0, 0.0));
            grounded.position.y = 10_000.0;
            for _ in 0..fps {
                for state in [&mut airborne, &mut grounded] {
                    state.step(
                        1.0 / fps as f32,
                        Bounds::new(10_000.0, 10_000.0),
                        Vec2::ZERO,
                        PhysicsConfig::default(),
                        None,
                    );
                }
            }
            assert!((airborne.velocity.x - 700.0).abs() < 0.01);
            assert!((airborne.velocity.y - 800.0).abs() < 0.01);
            assert!((grounded.velocity.x - 200.0).abs() < 0.01);
            assert_eq!(grounded.velocity.y, 0.0);
        }
    }

    #[test]
    fn simulation_velocity_is_bounded_in_both_directions_and_zero_stops_an_axis() {
        for sign in [-1.0, 1.0] {
            for limit in [Vec2::new(250.0, 125.0), Vec2::ZERO] {
                let mut state = airborne_state(Vec2::new(sign * 1000.0, sign * 1000.0));
                state.step(
                    0.1,
                    Bounds::new(10_000.0, 10_000.0),
                    Vec2::ZERO,
                    PhysicsConfig {
                        max_velocity: limit,
                        ..friction_config(0.0, 0.0)
                    },
                    None,
                );
                assert_eq!(state.velocity, Vec2::new(sign * limit.x, sign * limit.y));
            }
        }
    }

    #[test]
    fn zero_drag_velocity_limit_stops_release_on_that_axis() {
        let mut state = PhysicsState::default();
        state.start_drag(Vec2::ZERO);
        state.drag_to(
            Vec2::new(100.0, 100.0),
            0.1,
            Bounds::new(200.0, 200.0),
            Vec2::ZERO,
            Vec2::new(0.0, 250.0),
        );
        state.end_drag();
        assert_eq!(state.velocity, Vec2::new(0.0, 250.0));
    }

    #[test]
    fn empty_and_new_physics_configs_use_acceleration_defaults() {
        let empty: PhysicsConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, PhysicsConfig::default());
        assert_eq!(empty.gravity, 800.0);
        assert_eq!(empty.air_friction, 100.0);
        assert_eq!(empty.ground_friction, 500.0);
        assert_eq!(empty.drag, 1.0);
        assert_eq!(empty.min_velocity, 0.0);

        let explicit: PhysicsConfig =
            serde_json::from_str(r#"{"airFriction":0,"drag":1}"#).unwrap();
        assert_eq!(explicit.air_friction, 0.0);
        assert_eq!(explicit.ground_friction, 500.0);
        assert_eq!(explicit.gravity, 800.0);
        assert_eq!(explicit.min_velocity, 0.0);
    }

    #[test]
    fn legacy_physics_configs_retain_original_damping_and_defaults() {
        let config: PhysicsConfig = serde_json::from_str(r#"{"drag":0.98}"#).unwrap();
        assert_eq!(config.gravity, 1000.0);
        assert_eq!(config.air_friction, 0.0);
        assert_eq!(config.ground_friction, 0.0);
        assert_eq!(config.drag, 0.98);
        assert_eq!(config.min_velocity, 5.0);

        let mut state = airborne_state(Vec2::new(100.0, 0.0));
        state.step(
            1.0 / 60.0,
            Bounds::new(10_000.0, 10_000.0),
            Vec2::ZERO,
            config,
            None,
        );
        assert!((state.velocity.x - 98.0).abs() < 0.001);
        assert!((state.velocity.y - 1000.0 / 60.0 * 0.98).abs() < 0.001);

        let explicit: PhysicsConfig =
            serde_json::from_str(r#"{"gravity":600,"drag":0.9,"minVelocity":3}"#).unwrap();
        assert_eq!(explicit.gravity, 600.0);
        assert_eq!(explicit.drag, 0.9);
        assert_eq!(explicit.min_velocity, 3.0);
        assert_eq!(explicit.air_friction, 0.0);
        assert_eq!(explicit.ground_friction, 0.0);
    }

    #[test]
    fn serialized_legacy_and_new_configs_round_trip_without_changing_behavior() {
        let legacy: PhysicsConfig =
            serde_json::from_str(r#"{"gravity":600,"drag":0.9,"minVelocity":3}"#).unwrap();
        for config in [legacy, PhysicsConfig::default(), friction_config(0.0, 0.0)] {
            let serialized = serde_json::to_string(&config).unwrap();
            assert!(serialized.contains("\"airFriction\":"));
            assert!(serialized.contains("\"groundFriction\":"));
            assert_eq!(
                serde_json::from_str::<PhysicsConfig>(&serialized).unwrap(),
                config
            );
        }
    }
}
