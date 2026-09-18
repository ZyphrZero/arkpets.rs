use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWindowCollectionBehavior};

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
    let windows = app.windows();
    let visible: Vec<_> = (0..windows.count())
        .map(|index| windows.objectAtIndex(index))
        .filter(|window| window.isVisible())
        .collect();
    let applied = app.setActivationPolicy(policy);
    // AppKit can order windows out during an activation-policy transition.
    // Restore only windows that were already visible, without taking focus.
    for window in visible {
        if !window.isVisible() {
            window.orderFrontRegardless();
        }
    }
    if !applied {
        return Err("NSApplication refused the requested activation policy".into());
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
    set_application_visible(show)?;
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
