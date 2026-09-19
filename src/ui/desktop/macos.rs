use eframe::egui::Pos2;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSScreen};
use objc2_foundation::NSPoint;
use std::io;

pub(super) fn cursor_position(zoom_factor: f32) -> io::Result<Pos2> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| io::Error::other("AppKit cursor sampling requires the main thread"))?;
    // The first screen defines the desktop origin. mainScreen follows the key
    // window and would change the Y origin when dragging to another display.
    let screens = NSScreen::screens(mtm);
    let primary = screens
        .firstObject()
        .ok_or_else(|| io::Error::other("No macOS screen is available"))?;
    screen_position(
        NSEvent::mouseLocation(),
        primary.frame().size.height,
        zoom_factor,
    )
}

fn screen_position(point: NSPoint, primary_height: f64, zoom_factor: f32) -> io::Result<Pos2> {
    // AppKit supplies logical points with Y upwards. winit flips Y and applies
    // the window's backing scale; egui divides by that same scale times zoom.
    // The backing scale cancels out, including on Retina displays.
    let position = Pos2::new(point.x as f32, (primary_height - point.y) as f32) / zoom_factor;
    if !position.is_finite()
        || !primary_height.is_finite()
        || primary_height <= 0.0
        || !zoom_factor.is_finite()
        || zoom_factor <= 0.0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid macOS screen cursor coordinates or scale",
        ));
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_coordinates_preserve_negative_displays_and_apply_zoom_once() {
        for (point, expected) in [
            (NSPoint::new(300.0, 700.0), Pos2::new(300.0, 200.0)),
            (NSPoint::new(-400.0, 1100.0), Pos2::new(-400.0, -200.0)),
            (NSPoint::new(1500.0, -100.0), Pos2::new(1500.0, 1000.0)),
        ] {
            for zoom in [0.5, 1.0, 1.5, 2.0] {
                assert_eq!(
                    screen_position(point, 900.0, zoom).unwrap(),
                    expected / zoom
                );
            }
        }
    }

    #[test]
    fn invalid_native_coordinates_are_reported() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(screen_position(NSPoint::new(value, 0.0), 900.0, 1.0).is_err());
            assert!(screen_position(NSPoint::new(0.0, value), 900.0, 1.0).is_err());
            assert!(screen_position(NSPoint::new(0.0, 0.0), value, 1.0).is_err());
        }
        for zoom in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(screen_position(NSPoint::new(0.0, 0.0), 900.0, zoom).is_err());
        }
    }
}
