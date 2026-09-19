#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub const SUPPORTED: bool = cfg!(any(windows, target_os = "linux", target_os = "macos"));

pub fn label() -> &'static str {
    if cfg!(target_os = "macos") {
        "在 Dock 和应用切换器中显示"
    } else {
        "在任务栏和 Alt+Tab 中显示桌宠"
    }
}

pub fn set_visible(show: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        super::platform::set_desktop_taskbar(show)
    }
    #[cfg(target_os = "linux")]
    {
        linux::set_visible(show).map_err(|error| format!("X11 shell visibility: {error}"))
    }
    #[cfg(target_os = "macos")]
    {
        macos::set_visible(show)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = show;
        Err("Shell visibility is not supported on this platform".into())
    }
}

#[cfg(target_os = "macos")]
pub fn set_application_visible(show: bool) -> Result<(), String> {
    macos::set_application_visible(show)
}

#[cfg(target_os = "macos")]
pub fn initialize_application() -> Result<(), String> {
    macos::initialize_application()
}
