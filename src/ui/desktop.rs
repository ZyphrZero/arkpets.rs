use arkpets::{Bounds, Direction, PetConfig, PhysicsState, Vec2 as PetVec2};
use eframe::egui::{Pos2, Rect, Vec2};
use std::{io, time::Instant};

pub struct DesktopMotion {
    pub body: PhysicsState,
    pub work_area: Rect,
    pub last_tick: Instant,
    pub last_window_position: Option<Pos2>,
    pub next_work_area_update: Instant,
    #[cfg(windows)]
    pub appearance_ready: bool,
    pub taskbar_visibility: Option<bool>,
}

impl DesktopMotion {
    pub fn new(work_area: Rect) -> Self {
        let mut body = PhysicsState::default();
        body.position = PetVec2::new(work_area.width() * 0.5, work_area.height() * 0.15);
        Self {
            body,
            work_area,
            last_tick: Instant::now(),
            last_window_position: None,
            next_work_area_update: Instant::now(),
            #[cfg(windows)]
            appearance_ready: false,
            taskbar_visibility: None,
        }
    }

    pub fn screen_position(&self) -> Pos2 {
        self.work_area.min + Vec2::new(self.body.position.x, self.body.position.y)
    }

    pub fn local_cursor(&self, cursor: Pos2) -> PetVec2 {
        let position = cursor - self.work_area.min;
        PetVec2::new(position.x, position.y)
    }

    pub fn set_work_area(&mut self, area: Rect, size: Vec2) {
        let position = self.screen_position() - area.min;
        self.work_area = area;
        let max =
            Bounds::new(area.width(), area.height()).max_position(PetVec2::new(size.x, size.y));
        self.body.position =
            PetVec2::new(position.x.clamp(0.0, max.x), position.y.clamp(0.0, max.y));
        if self.body.position.x != position.x {
            self.body.velocity.x = 0.0;
        }
        if self.body.position.y != position.y {
            self.body.velocity.y = 0.0;
        }
    }

    pub fn step(
        &mut self,
        dt: f32,
        size: Vec2,
        config: &PetConfig,
        moving: bool,
        direction: &mut Direction,
    ) {
        let bounds = Bounds::new(self.work_area.width(), self.work_area.height());
        self.body.step(
            dt,
            bounds,
            PetVec2::new(size.x, size.y),
            config.physics,
            (moving && config.allow_walk).then_some((*direction, config.walk_speed)),
        );
        if moving && !self.body.dragging {
            if self.body.position.x <= 0.0 {
                *direction = Direction::Right;
            } else if self.body.position.x >= (bounds.width - size.x).max(0.0) {
                *direction = Direction::Left;
            }
        }
    }
}

/// Read screen coordinates without reusing a moving window's cached pointer position.
pub fn cursor_position(pixels_per_point: f32, fallback: Option<Pos2>) -> io::Result<Option<Pos2>> {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pixels_per_point must be finite and positive",
        ));
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

        let _ = fallback;
        let mut position = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut position) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(
            Pos2::new(position.x as f32, position.y as f32) / pixels_per_point,
        ))
    }
    #[cfg(not(windows))]
    {
        Ok(fallback)
    }
}

fn movement_area(monitor: Rect, mut work: Rect, bottom_height: Option<u32>, scale: f32) -> Rect {
    if let Some(height) = bottom_height {
        // A manual height replaces the bottom inset, measured in physical pixels.
        work.max.y = (monitor.max.y - height as f32 / scale).max(work.min.y + 1.0);
    }
    work
}

pub fn work_area(
    monitor_size: Vec2,
    window_position: Pos2,
    pixels_per_point: f32,
    bottom_height: Option<u32>,
) -> io::Result<Rect> {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pixels_per_point must be finite and positive",
        ));
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::POINT,
            Graphics::Gdi::{
                GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
            },
        };
        let point = POINT {
            x: (window_position.x * pixels_per_point) as i32,
            y: (window_position.y * pixels_per_point) as i32,
        };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let _ = monitor_size;
        if unsafe { GetMonitorInfoW(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST), &mut info) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let rect = |r: windows_sys::Win32::Foundation::RECT| {
            Rect::from_min_max(
                Pos2::new(r.left as f32, r.top as f32) / pixels_per_point,
                Pos2::new(r.right as f32, r.bottom as f32) / pixels_per_point,
            )
        };
        Ok(movement_area(
            rect(info.rcMonitor),
            rect(info.rcWork),
            bottom_height,
            pixels_per_point,
        ))
    }
    #[cfg(not(windows))]
    {
        let _ = window_position;
        let monitor = Rect::from_min_size(Pos2::ZERO, monitor_size);
        Ok(movement_area(
            monitor,
            monitor,
            bottom_height,
            pixels_per_point,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_floor_replaces_system_inset_in_physical_pixels() {
        let monitor = Rect::from_min_max(Pos2::new(-1280.0, -200.0), Pos2::new(0.0, 520.0));
        let work = Rect::from_min_max(Pos2::new(-1240.0, -180.0), Pos2::new(0.0, 488.0));
        assert_eq!(movement_area(monitor, work, None, 1.5), work);
        for (height, expected_floor) in [(0, 520.0), (48, 488.0), (120, 440.0)] {
            let area = movement_area(monitor, work, Some(height), 1.5);
            assert_eq!(area.min, work.min);
            assert_eq!(area.right(), work.right());
            assert_eq!(area.bottom(), expected_floor);
            let mut motion = DesktopMotion::new(area);
            let size = Vec2::new(150.0, 200.0);
            let mut direction = Direction::Right;
            for _ in 0..180 {
                motion.step(
                    1.0 / 60.0,
                    size,
                    &PetConfig::default(),
                    true,
                    &mut direction,
                );
            }
            assert_eq!(motion.screen_position().y + size.y, expected_floor);
        }
        assert_eq!(
            movement_area(monitor, work, Some(u32::MAX), 1.5).height(),
            1.0
        );
    }

    #[test]
    fn changing_work_area_preserves_position_and_clamps_without_a_physics_tick() {
        let area = Rect::from_min_size(Pos2::new(-1280.0, 0.0), Vec2::new(1280.0, 720.0));
        let mut motion = DesktopMotion::new(area);
        let size = Vec2::new(150.0, 200.0);
        motion.body.position = PetVec2::new(500.0, 520.0);
        motion.body.velocity = PetVec2::new(20.0, 30.0);
        let raised = Rect::from_min_max(Pos2::new(-1240.0, 20.0), Pos2::new(0.0, 640.0));
        motion.set_work_area(raised, size);
        assert_eq!(motion.screen_position(), Pos2::new(-780.0, 440.0));
        assert_eq!(motion.body.velocity, PetVec2::new(20.0, 0.0));
        motion.set_work_area(area, size);
        assert_eq!(motion.screen_position(), Pos2::new(-780.0, 440.0));
        for _ in 0..120 {
            motion.step(
                1.0 / 60.0,
                size,
                &PetConfig::default(),
                false,
                &mut Direction::Right,
            );
        }
        assert_eq!(motion.screen_position().y, 520.0);
    }

    #[test]
    fn desktop_falls_walks_turns_and_retains_drag_release_velocity() {
        let area = Rect::from_min_size(Pos2::new(-1280.0, 0.0), Vec2::new(1280.0, 720.0));
        let mut motion = DesktopMotion::new(area);
        let config = PetConfig::default();
        let size = Vec2::new(150.0, 200.0);
        let mut direction = Direction::Right;
        let start = motion.body.position;
        for _ in 0..180 {
            motion.step(1.0 / 60.0, size, &config, false, &mut direction);
        }
        assert_eq!(motion.body.position.y, 520.0);
        assert!(motion.body.position.y > start.y);
        let start_x = motion.body.position.x;
        motion.step(0.1, size, &config, true, &mut direction);
        assert!(motion.body.position.x > start_x);
        motion.body.position.x = 1130.0;
        motion.step(0.1, size, &config, true, &mut direction);
        assert_eq!(direction, Direction::Left);
        let cursor = motion.local_cursor(motion.screen_position());
        motion.body.start_drag(cursor);
        motion.body.drag_to(
            PetVec2::new(cursor.x - 50.0, 100.0),
            0.1,
            Bounds::new(1280.0, 720.0),
            PetVec2::new(150.0, 200.0),
            config.physics.max_velocity,
        );
        motion.body.end_drag();
        assert!(motion.body.velocity.x < 0.0);
        assert!(motion.body.velocity.y < 0.0);
    }

    #[test]
    fn stationary_screen_cursor_does_not_move_or_throw_the_window() {
        let area = Rect::from_min_size(Pos2::new(-1280.0, 0.0), Vec2::new(1280.0, 720.0));
        let mut motion = DesktopMotion::new(area);
        let bounds = Bounds::new(area.width(), area.height());
        let size = PetVec2::new(150.0, 200.0);
        let limit = PetConfig::default().physics.max_velocity;
        let start_cursor = motion.screen_position() + Vec2::new(35.0, 45.0);
        motion.body.start_drag(motion.local_cursor(start_cursor));
        let screen_cursor = start_cursor + Vec2::new(80.0, 60.0);
        motion
            .body
            .drag_to(motion.local_cursor(screen_cursor), 0.1, bounds, size, limit);
        let position = motion.screen_position();
        for _ in 0..20 {
            motion.body.drag_to(
                motion.local_cursor(screen_cursor),
                0.000_001,
                bounds,
                size,
                limit,
            );
            assert_eq!(motion.screen_position(), position);
            assert_eq!(motion.body.velocity, PetVec2::ZERO);
        }
        motion.body.end_drag();
        assert_eq!(motion.body.velocity, PetVec2::ZERO);
    }

    #[test]
    fn cursor_position_rejects_an_invalid_coordinate_scale() {
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let error = cursor_position(scale, Some(Pos2::ZERO)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn cursor_position_uses_the_supplied_fallback_outside_windows() {
        let position = Pos2::new(-40.0, 70.0);
        assert_eq!(
            cursor_position(2.0, Some(position)).unwrap(),
            Some(position)
        );
        assert_eq!(cursor_position(2.0, None).unwrap(), None);
    }
}
