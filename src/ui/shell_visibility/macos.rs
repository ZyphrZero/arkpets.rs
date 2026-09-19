use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSImage, NSWindowCollectionBehavior,
};
use objc2_foundation::{ns_string, NSData, NSProcessInfo};

pub fn initialize_application() -> Result<(), String> {
    let main_thread = MainThreadMarker::new().ok_or("AppKit requires the main thread")?;
    NSProcessInfo::processInfo().setProcessName(ns_string!("ArkPets"));
    let data = NSData::with_bytes(include_bytes!("../../../assets/arkpets-icon.png"));
    let icon = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or("Cannot decode the ArkPets application icon")?;
    // SAFETY: A valid, retained image is supplied; NSApplication retains it.
    unsafe { NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&icon)) };
    Ok(())
}

pub fn set_application_visible(show: bool) -> Result<(), String> {
    let main_thread = MainThreadMarker::new().ok_or("AppKit requires the main thread")?;
    let app = NSApplication::sharedApplication(main_thread);
    let policy = if show {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    if app.activationPolicy() == policy {
        return Ok(());
    }
    let was_active = app.isActive();
    let key_window = app.keyWindow();
    let windows = app.windows();
    let visible: Vec<_> = (0..windows.count())
        .map(|index| windows.objectAtIndex(index))
        .filter(|window| window.isVisible())
        .collect();
    let applied = app.setActivationPolicy(policy);
    // AppKit can order windows out during an activation-policy transition.
    // Restore only windows that were already visible, without taking focus.
    for window in &visible {
        if !window.isVisible() {
            window.orderFrontRegardless();
        }
    }
    if !applied {
        return Err("NSApplication refused the requested activation policy".into());
    }
    if was_active {
        // Changing Dock policy can asynchronously yield activation and lower the
        // launcher behind other apps even while isVisible remains true. Restore
        // its foreground state on the main queue after AppKit processes the change.
        let previous = MainThreadBound::new((visible, key_window), main_thread);
        DispatchQueue::main().exec_async(move || {
            let main_thread = MainThreadMarker::new().expect("main dispatch queue");
            let app = NSApplication::sharedApplication(main_thread);
            if app.activationPolicy() != policy {
                return;
            }
            let (visible, key_window) = previous.into_inner(main_thread);
            app.unhideWithoutActivation();
            // Keep compatibility with macOS versions before NSApplication.activate.
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
            let windows = app.windows();
            for window in visible {
                // The snapshot can retain a closed NSWindow in app.windows().
                // Respect a close/hide that happened after the policy change.
                if windows.containsObject(&window) && window.isVisible() && !window.isMiniaturized()
                {
                    window.orderFrontRegardless();
                }
            }
            if let Some(window) = key_window {
                if window.isVisible() && !window.isMiniaturized() {
                    window.makeKeyWindow();
                }
            }
        });
    }
    Ok(())
}

pub fn set_visible(show: bool) -> Result<(), String> {
    let main_thread = MainThreadMarker::new().ok_or("AppKit requires the main thread")?;
    let app = NSApplication::sharedApplication(main_thread);
    let windows = app.windows();
    let window = (0..windows.count())
        .map(|index| windows.objectAtIndex(index))
        .find(|window| window.title().to_string() == "ArkPets - \u{684c}\u{5ba0}")
        .ok_or("Desktop pet NSWindow was not found")?;
    let mut behavior = window.collectionBehavior();
    behavior.remove(
        NSWindowCollectionBehavior::Managed
            | NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::ParticipatesInCycle
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    behavior.insert(if show {
        NSWindowCollectionBehavior::Managed | NSWindowCollectionBehavior::ParticipatesInCycle
    } else {
        NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::IgnoresCycle
    });
    window.setCollectionBehavior(behavior);
    window.setExcludedFromWindowsMenu(!show);
    window.setHidesOnDeactivate(false);
    window.setCanHide(false);
    Ok(())
}
