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
#[serde(rename_all = "camelCase", default)]
pub struct PhysicsConfig {
    pub gravity: f32,
    pub drag: f32,
    pub max_velocity: Vec2,
    pub min_velocity: f32,
    pub bounce_damping: f32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        // These values match the web implementation. The desktop defaults are
        // close enough for the same simulation and can be configured per app.
        Self {
            gravity: 1000.0,
            drag: 0.98,
            max_velocity: Vec2::new(1000.0, 1000.0),
            min_velocity: 5.0,
            bounce_damping: 0.7,
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
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.1);
        let max = bounds.max_position(size);

        if !self.dragging {
            self.velocity.y += config.gravity * dt;
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
}
