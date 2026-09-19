//! Native regression for stale window-relative pointer positions during dragging.

#[cfg(all(target_os = "macos", not(test)))]
#[allow(dead_code)]
mod application {
    include!("../../src/ui/mod.rs");

    pub fn verify() {
        use eframe::egui::{Pos2, Rect, Vec2};
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSEvent, NSScreen, NSWindow, NSWindowStyleMask};
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let mtm = MainThreadMarker::new().expect("main thread");
        let _app = NSApplication::sharedApplication(mtm);
        let screens = NSScreen::screens(mtm);
        let primary = screens.firstObject().expect("a graphical desktop");
        let height = primary.frame().size.height;
        let native_scale = primary.backingScaleFactor() as f32;
        // SAFETY: AppKit is initialized on the main thread; the window is retained
        // for the entire test and is hidden before it is released.
        let window = unsafe { NSWindow::new(mtm) };
        window.setStyleMask(NSWindowStyleMask::Borderless);
        window.setFrame_display(
            NSRect::new(NSPoint::new(100.0, 200.0), NSSize::new(160.0, 200.0)),
            true,
        );
        window.orderFrontRegardless();
        let point = NSEvent::mouseLocation();
        let screen_cursor = Pos2::new(point.x as f32, (height - point.y) as f32);
        let window_origin = || {
            let frame = window.frame();
            Pos2::new(
                frame.origin.x as f32,
                (height - frame.origin.y - frame.size.height) as f32,
            )
        };
        let origin = window_origin();
        let cached_local = screen_cursor - origin;
        let size = Vec2::new(160.0, 200.0);
        let mut motion = desktop::DesktopMotion::new(Rect::from_min_max(
            Pos2::new(-10000.0, -10000.0),
            Pos2::new(10000.0, 10000.0),
        ));
        motion.body.position = motion.local_cursor(origin);
        motion.start_drag(screen_cursor, size);
        for shift in [Vec2::new(80.0, 60.0), Vec2::new(-40.0, -30.0), Vec2::ZERO] {
            let moved = origin + shift;
            window.setFrameOrigin(NSPoint::new(
                moved.x as f64,
                height - moved.y as f64 - 200.0,
            ));
            let observed = window_origin();
            assert!(
                observed.distance(moved) < 0.01,
                "native window did not move"
            );
            let stale = observed + cached_local;
            let actual = desktop::cursor_position(native_scale, 1.0, Some(stale))
                .unwrap()
                .expect("global cursor");
            assert!(
                actual.distance(screen_cursor) < 0.01,
                "window movement changed the sampled cursor: {screen_cursor:?} -> {actual:?}; \
                 keep the mouse still during this test"
            );
            motion.drag_to(
                actual,
                0.001,
                size,
                arkpets::PetConfig::default().physics.max_velocity,
            );
            assert!(motion.screen_position().distance(origin) < 0.01);
            assert_eq!(motion.body.velocity, arkpets::Vec2::ZERO);
        }
        motion.body.end_drag();
        assert_eq!(motion.body.velocity, arkpets::Vec2::ZERO);
        for zoom in [0.5, 1.0, 1.5, 2.0] {
            let actual = desktop::cursor_position(native_scale * zoom, zoom, None)
                .unwrap()
                .expect("cursor outside the window still has a global position");
            assert!(actual.distance(screen_cursor / zoom) < 0.01);
        }
        window.orderOut(None);
        println!("PASS: native window moves do not change the global cursor, pet position, or release velocity");
    }
}

fn main() {
    #[cfg(all(target_os = "macos", not(test)))]
    application::verify();
    #[cfg(not(target_os = "macos"))]
    panic!("This native drag verification requires macOS");
}
