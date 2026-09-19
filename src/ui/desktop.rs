#[cfg(target_os = "macos")]
mod macos;
mod regions;

use arkpets::{
    interaction::DirectionGesture, Bounds, Direction, PetConfig, PhysicsState, Vec2 as PetVec2,
};
use eframe::egui::{Context, Pos2, Rect, Vec2};
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
    pub gesture: DirectionGesture,
    pub pixels_per_point: f32,
    pub size: Vec2,
    areas: Vec<Rect>,
    drag_offset: Vec2,
    initial_position: Option<[f32; 2]>,
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
            gesture: DirectionGesture::default(),
            pixels_per_point: 1.0,
            size: Vec2::ZERO,
            areas: vec![work_area],
            drag_offset: Vec2::ZERO,
            initial_position: None,
        }
    }

    pub fn screen_position(&self) -> Pos2 {
        self.work_area.min + Vec2::new(self.body.position.x, self.body.position.y)
    }

    pub fn local_cursor(&self, cursor: Pos2) -> PetVec2 {
        let position = cursor - self.work_area.min;
        PetVec2::new(position.x, position.y)
    }

    pub fn deploy(
        areas: Vec<Rect>,
        primary: Rect,
        size: Vec2,
        position: [f32; 2],
        scale: f32,
    ) -> io::Result<Self> {
        if position
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "initial position must be between 0 and 1",
            ));
        }
        let mut motion = Self::new(primary);
        motion.pixels_per_point = scale;
        motion.body.position = PetVec2::new(
            (primary.width() - size.x).max(0.0) * position[0],
            (primary.height() - size.y).max(0.0) * position[1],
        );
        motion.set_areas(areas, size, scale)?;
        motion.initial_position = Some(position);
        Ok(motion)
    }

    /// The launcher and the new pet can be on displays with different DPI.
    /// Apply percentages once using the pet viewport's actual scale and size.
    pub fn finish_deployment(&mut self, primary: Rect, size: Vec2) {
        if let Some(position) = self.initial_position.take() {
            let requested = primary.min
                + Vec2::new(
                    (primary.width() - size.x).max(0.0) * position[0],
                    (primary.height() - size.y).max(0.0) * position[1],
                );
            if let Some(projected) = regions::project(requested, size, &self.areas) {
                self.body.position = self.local_cursor(projected);
            }
        }
    }

    pub fn set_areas(&mut self, areas: Vec<Rect>, size: Vec2, scale: f32) -> io::Result<()> {
        let position = self.screen_position() * (self.pixels_per_point / scale);
        let projected = regions::project(position, size, &areas)
            .ok_or_else(|| io::Error::other("桌宠尺寸超出可用屏幕区域，请减小显示比例"))?;
        let area = areas
            .iter()
            .copied()
            .reduce(Rect::union)
            .ok_or_else(|| io::Error::other("未找到可用显示器"))?;
        let ratio = self.pixels_per_point / scale;
        self.body.velocity.x *= ratio;
        self.body.velocity.y *= ratio;
        self.work_area = area;
        self.areas = areas;
        self.size = size;
        self.pixels_per_point = scale;
        self.body.position = self.local_cursor(projected);
        if (projected.x - position.x).abs() > 0.01 {
            self.body.velocity.x = 0.0;
        }
        if (projected.y - position.y).abs() > 0.01 {
            self.body.velocity.y = 0.0;
        }
        Ok(())
    }

    pub fn start_drag(&mut self, cursor: Pos2, size: Vec2) {
        self.drag_offset = (cursor - self.screen_position()) / size;
        self.body.start_drag(self.local_cursor(cursor));
        self.gesture.start_drag(cursor.x * self.pixels_per_point);
    }

    pub fn drag_to(&mut self, cursor: Pos2, dt: f32, size: Vec2, limit: PetVec2) {
        let old = self.screen_position();
        if let Some(position) =
            regions::project(cursor - self.drag_offset * size, size, &self.areas)
        {
            self.body.position = self.local_cursor(position);
            self.body.velocity = if dt.is_finite() && dt > 0.0 {
                PetVec2::new(
                    ((position.x - old.x) / dt).clamp(-limit.x, limit.x),
                    ((position.y - old.y) / dt).clamp(-limit.y, limit.y),
                )
            } else {
                PetVec2::ZERO
            };
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
        let before = self.screen_position();
        let bounds = Bounds::new(self.work_area.width(), self.work_area.height());
        self.body.step(
            dt,
            bounds,
            PetVec2::new(size.x, size.y),
            config.physics,
            (moving && config.allow_walk).then_some((*direction, config.walk_speed)),
        );
        let requested = self.screen_position();
        let position = regions::travel(before, requested, size, &self.areas);
        self.body.position = self.local_cursor(position);
        if (position.x - requested.x).abs() > 0.01 {
            self.body.velocity.x = -self.body.velocity.x * config.physics.bounce_damping;
            if moving && !self.body.dragging {
                *direction = if requested.x > position.x {
                    Direction::Left
                } else {
                    Direction::Right
                };
            }
        }
        if (position.y - requested.y).abs() > 0.01 {
            self.body.velocity.y = 0.0;
        }
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
pub fn cursor_position(
    pixels_per_point: f32,
    zoom_factor: f32,
    fallback: Option<Pos2>,
) -> io::Result<Option<Pos2>> {
    if !pixels_per_point.is_finite()
        || pixels_per_point <= 0.0
        || !zoom_factor.is_finite()
        || zoom_factor <= 0.0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pixels_per_point and zoom_factor must be finite and positive",
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
    #[cfg(target_os = "macos")]
    {
        let _ = fallback;
        macos::cursor_position(zoom_factor).map(Some)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
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

fn work_area(monitor: Rect, pixels_per_point: f32, bottom_height: Option<u32>) -> io::Result<Rect> {
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
        let window_position = monitor.center();
        let point = POINT {
            x: (window_position.x * pixels_per_point) as i32,
            y: (window_position.y * pixels_per_point) as i32,
        };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
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
        Ok(movement_area(
            monitor,
            monitor,
            bottom_height,
            pixels_per_point,
        ))
    }
}

pub fn movement_areas(
    ctx: &Context,
    scale: f32,
    bottom: Option<u32>,
    multiple: bool,
) -> io::Result<(Vec<Rect>, Rect)> {
    let mut monitors = egui_winit::monitors::MonitorSnapshot::get(ctx)
        .ok_or_else(|| io::Error::other("显示器信息尚未就绪"))?
        .monitors;
    monitors.sort_by_key(|monitor| !monitor.primary);
    if !multiple {
        monitors.truncate(1);
    }
    let areas: Vec<_> = monitors
        .into_iter()
        .map(|monitor| work_area(monitor.rect / scale, scale, bottom))
        .collect::<io::Result<_>>()?;
    let primary = *areas
        .first()
        .ok_or_else(|| io::Error::other("未找到可用显示器"))?;
    Ok((areas, primary))
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
        motion.set_areas(vec![raised], size, 1.0).unwrap();
        assert_eq!(motion.screen_position(), Pos2::new(-780.0, 440.0));
        assert_eq!(motion.body.velocity, PetVec2::new(20.0, 0.0));
        motion.set_areas(vec![area], size, 1.0).unwrap();
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
            let error = cursor_position(scale, 1.0, Some(Pos2::ZERO)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            let error = cursor_position(2.0, scale, Some(Pos2::ZERO)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn deployment_uses_available_range_and_dpi_changes_preserve_physical_position() {
        let primary = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1040.0));
        let secondary = Rect::from_min_size(Pos2::new(-1280.0, -200.0), Vec2::new(1280.0, 1024.0));
        let size = Vec2::new(200.0, 200.0);
        let mut motion =
            DesktopMotion::deploy(vec![primary, secondary], primary, size, [1.0, 1.0], 1.0)
                .unwrap();
        assert_eq!(motion.screen_position(), Pos2::new(1720.0, 840.0));
        motion.body.position = motion.local_cursor(Pos2::new(-600.0, 200.0));
        let physical = motion.screen_position();
        motion
            .set_areas(vec![primary / 1.5, secondary / 1.5], size, 1.5)
            .unwrap();
        assert!(motion.screen_position().distance(physical / 1.5) < 0.001);
        motion.set_areas(vec![primary / 1.5], size, 1.5).unwrap();
        assert!(primary.contains_rect(Rect::from_min_size(motion.screen_position(), size) * 1.5));
        assert!(DesktopMotion::deploy(vec![primary], primary, size, [f32::NAN, 0.0], 1.0).is_err());
        assert!(DesktopMotion::deploy(vec![primary], primary, size, [-0.1, 0.0], 1.0).is_err());
    }

    #[test]
    fn deployment_uses_pet_dpi_once_and_backend_recreation_keeps_the_position() {
        let monitor = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let size = Vec2::new(200.0, 200.0);
        let mut motion =
            DesktopMotion::deploy(vec![monitor / 1.5], monitor / 1.5, size, [1.0, 0.5], 1.5)
                .unwrap();
        motion.set_areas(vec![monitor], size, 1.0).unwrap();
        motion.finish_deployment(monitor, size);
        assert_eq!(motion.screen_position(), Pos2::new(1720.0, 440.0));
        motion.body.position = PetVec2::new(300.0, 400.0);
        motion.last_window_position = None;
        motion.finish_deployment(monitor, size);
        assert_eq!(motion.screen_position(), Pos2::new(300.0, 400.0));
    }

    #[test]
    fn multi_display_drag_keeps_anchor_and_cross_display_walk_falls_to_each_floor() {
        let left = Rect::from_min_max(Pos2::new(-1000.0, 0.0), Pos2::new(0.0, 800.0));
        let right = Rect::from_min_max(Pos2::new(0.0, 200.0), Pos2::new(1200.0, 1000.0));
        let size = Vec2::new(100.0, 100.0);
        let config = PetConfig {
            walk_speed: 200.0,
            ..Default::default()
        };
        let mut motion =
            DesktopMotion::deploy(vec![left, right], left, size, [1.0, 1.0], 1.0).unwrap();
        let mut direction = Direction::Right;
        for _ in 0..120 {
            motion.step(1.0 / 60.0, size, &config, true, &mut direction);
        }
        assert!(motion.screen_position().x > 200.0);
        assert!((motion.screen_position().y - 900.0).abs() < 0.01);
        let cursor = motion.screen_position() + Vec2::new(25.0, 30.0);
        motion.start_drag(cursor, size);
        let cursor = Pos2::new(-500.0, 300.0);
        motion.drag_to(cursor, 0.1, size, config.physics.max_velocity);
        assert_eq!(motion.screen_position(), cursor - Vec2::new(25.0, 30.0));
        motion.drag_to(cursor, 0.1, size, config.physics.max_velocity);
        assert_eq!(motion.body.velocity, PetVec2::ZERO);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn cursor_position_uses_the_supplied_fallback_without_native_sampling() {
        let position = Pos2::new(-40.0, 70.0);
        assert_eq!(
            cursor_position(2.0, 1.0, Some(position)).unwrap(),
            Some(position)
        );
        assert_eq!(cursor_position(2.0, 1.0, None).unwrap(), None);
    }
}
