//! A cached monitor snapshot for applications using deferred viewports.

use egui::{Context, Id, Pos2, Rect, Vec2};
use std::time::{Duration, Instant};
use winit::window::Window;

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    /// Virtual desktop coordinates in physical pixels, matching winit positions.
    pub rect: Rect,
    pub primary: bool,
}

#[derive(Clone, Debug)]
pub struct MonitorSnapshot {
    pub monitors: Vec<MonitorInfo>,
    pub updated: Instant,
}

impl MonitorSnapshot {
    pub fn get(ctx: &Context) -> Option<Self> {
        ctx.data_mut(|data| data.get_temp::<Self>(Id::NULL))
    }
}

pub(crate) fn update(ctx: &Context, window: &Window) {
    if MonitorSnapshot::get(ctx)
        .is_some_and(|snapshot| snapshot.updated.elapsed() < Duration::from_secs(1))
    {
        return;
    }
    let primary = window.primary_monitor();
    let monitors = window
        .available_monitors()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            MonitorInfo {
                rect: Rect::from_min_size(
                    Pos2::new(position.x as f32, position.y as f32),
                    Vec2::new(size.width as f32, size.height as f32),
                ),
                primary: primary.as_ref() == Some(&monitor),
            }
        })
        .collect();
    ctx.data_mut(|data| {
        data.insert_temp(
            Id::NULL,
            MonitorSnapshot {
                monitors,
                updated: Instant::now(),
            },
        )
    });
}
