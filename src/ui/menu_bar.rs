use std::cell::Cell;

use objc2::{
    define_class, msg_send, rc::Retained, sel, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSCellImagePosition, NSImage, NSMenu, NSMenuItem, NSSquareStatusItemLength, NSStatusBar,
    NSStatusItem,
};
use objc2_foundation::{ns_string, NSObject, NSObjectProtocol, NSSize};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Requests {
    pub show_launcher: bool,
    pub desktop_open: Option<bool>,
    pub quit: bool,
}

struct MenuTargetIvars {
    pending: Cell<Requests>,
    desktop_open: Cell<bool>,
    wake: Box<dyn Fn()>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; this class is main-thread only.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "ArkPetsMenuTarget"]
    #[ivars = MenuTargetIvars]
    struct MenuTarget;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(showLauncher:))]
        fn show_launcher(&self, _: &NSMenuItem) {
            self.request(|pending| pending.show_launcher = true);
        }

        #[unsafe(method(toggleDesktop:))]
        fn toggle_desktop(&self, _: &NSMenuItem) {
            let open = !self.ivars().desktop_open.get();
            self.ivars().desktop_open.set(open);
            self.request(|pending| pending.desktop_open = Some(open));
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _: &NSMenuItem) {
            self.request(|pending| pending.quit = true);
        }
    }
);

impl MenuTarget {
    fn request(&self, update: impl FnOnce(&mut Requests)) {
        let mut pending = self.ivars().pending.get();
        update(&mut pending);
        self.ivars().pending.set(pending);
        (self.ivars().wake)();
    }
}

/// Owned by NativeApp, outside the state captured by deferred egui viewports.
/// The action target retains only a wake callback, so it cannot form an App/Context cycle.
pub struct MenuBar {
    item: Retained<NSStatusItem>,
    target: Retained<MenuTarget>,
    desktop_item: Retained<NSMenuItem>,
}

impl MenuBar {
    pub fn new(wake: impl Fn() + 'static) -> Result<Self, String> {
        let main_thread =
            MainThreadMarker::new().ok_or("Menu bar requires the AppKit main thread")?;
        if !objc2::available!(macos = 11.0) {
            return Err("The native menu bar icon requires macOS 11 or later".into());
        }
        let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            ns_string!("pawprint.fill"),
            Some(ns_string!("ArkPets 桌宠")),
        )
        .ok_or("macOS could not load the pawprint.fill menu bar symbol")?;
        icon.setSize(NSSize::new(18.0, 18.0));
        icon.setTemplate(true);
        let target = MenuTarget::alloc(main_thread).set_ivars(MenuTargetIvars {
            pending: Cell::default(),
            desktop_open: Cell::new(false),
            wake: Box::new(wake),
        });
        // SAFETY: NSObject.init initializes the allocated superclass; all ivars are set above.
        let target: Retained<MenuTarget> = unsafe { msg_send![super(target), init] };
        let menu = NSMenu::initWithTitle(NSMenu::alloc(main_thread), ns_string!("ArkPets"));
        menu.setAutoenablesItems(false);
        let mut items = Vec::new();
        for (title, action) in [
            (ns_string!("显示主界面"), sel!(showLauncher:)),
            (ns_string!("启动桌宠"), sel!(toggleDesktop:)),
            (ns_string!("退出 ArkPets"), sel!(quit:)),
        ] {
            // SAFETY: Each selector is implemented above with NSMenuItem sender/void return.
            // MenuBar retains the target for the whole menu lifetime.
            let item = unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(main_thread),
                    title,
                    Some(action),
                    ns_string!(""),
                );
                item.setTarget(Some(&target));
                item
            };
            if items.len() == 2 {
                menu.addItem(&NSMenuItem::separatorItem(main_thread));
            }
            menu.addItem(&item);
            items.push(item);
        }
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
        let Some(button) = item.button(main_thread) else {
            NSStatusBar::systemStatusBar().removeStatusItem(&item);
            return Err("macOS did not create an ArkPets menu bar button".into());
        };
        button.setTitle(ns_string!(""));
        button.setImage(Some(&icon));
        button.setImagePosition(NSCellImagePosition::ImageOnly);
        button.setToolTip(Some(ns_string!("ArkPets 桌宠")));
        item.setMenu(Some(&menu));
        Ok(Self {
            item,
            target,
            desktop_item: items.remove(1),
        })
    }

    pub fn take_requests(&self) -> Requests {
        self.target.ivars().pending.take()
    }

    pub fn update(&self, desktop_open: bool, model_loaded: bool) {
        self.target.ivars().desktop_open.set(desktop_open);
        let title = if desktop_open {
            ns_string!("收起桌宠")
        } else {
            ns_string!("启动桌宠")
        };
        if &*self.desktop_item.title() != title {
            self.desktop_item.setTitle(title);
        }
        let enabled = model_loaded || desktop_open;
        if self.desktop_item.isEnabled() != enabled {
            self.desktop_item.setEnabled(enabled);
        }
    }
}

impl Drop for MenuBar {
    fn drop(&mut self) {
        self.item.setMenu(None);
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}
