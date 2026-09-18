use std::{io, mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{SetLastError, BOOL, E_INVALIDARG, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    },
    System::Threading::GetCurrentThreadId,
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            EnumThreadWindows, GetWindowLongW, GetWindowTextW, IsWindowVisible, SetWindowLongW,
            SetWindowPos, ShowWindow, GWL_EXSTYLE, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WM_NCCALCSIZE,
            WM_NCDESTROY, WM_STYLECHANGING, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
        },
    },
};

unsafe extern "system" fn desktop_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    show_in_taskbar: usize,
) -> LRESULT {
    if message == WM_STYLECHANGING && wparam as i32 == GWL_EXSTYLE {
        // winit rewrites extended styles when showing the window or changing
        // topmost/passthrough. Preserve the shell policy in those updates too.
        let styles = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
        styles.styleNew = taskbar_style(styles.styleNew, show_in_taskbar != 0);
    }
    if message == WM_NCCALCSIZE && wparam != 0 {
        // egui-winit enables winit's undecorated shadow, whose NCCALCSIZE adds
        // a one-pixel top border. This overlay has no non-client area at all.
        return 0;
    }
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(window, Some(desktop_window_proc), id);
        }
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

fn taskbar_style(style: u32, show: bool) -> u32 {
    if show {
        (style & !WS_EX_TOOLWINDOW) | WS_EX_APPWINDOW
    } else {
        (style & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopAppearance {
    Applied,
    Unsupported,
}

fn desktop_window() -> Result<HWND, String> {
    struct Search {
        title: Vec<u16>,
        window: HWND,
    }

    unsafe extern "system" fn find_window(window: HWND, parameter: LPARAM) -> BOOL {
        // EnumThreadWindows invokes this synchronously with our live stack value.
        let search = unsafe { &mut *(parameter as *mut Search) };
        let mut title = [0_u16; 64];
        let length = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
        if length > 0 && title[..length as usize] == search.title {
            search.window = window;
        }
        1
    }

    let mut search = Search {
        title: "ArkPets - \u{684c}\u{5ba0}".encode_utf16().collect(),
        window: ptr::null_mut(),
    };
    // Restrict discovery to the calling GUI thread, excluding other app instances.
    if unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(find_window),
            &mut search as *mut Search as LPARAM,
        )
    } == 0
    {
        return Err(format!(
            "EnumThreadWindows failed: {}",
            io::Error::last_os_error()
        ));
    }
    if search.window.is_null() {
        return Err("Desktop pet window was not found on the current GUI thread".into());
    }
    Ok(search.window)
}

fn apply_taskbar_style(window: HWND, show_in_taskbar: bool) -> Result<(), String> {
    unsafe { SetLastError(0) };
    let style = unsafe { GetWindowLongW(window, GWL_EXSTYLE) };
    if style == 0 && io::Error::last_os_error().raw_os_error() != Some(0) {
        return Err(format!(
            "GetWindowLongW failed: {}",
            io::Error::last_os_error()
        ));
    }
    // Updating the subclass data first prevents the previous policy from
    // overriding our own style change. WM_NCDESTROY removes the subclass.
    if unsafe {
        SetWindowSubclass(
            window,
            Some(desktop_window_proc),
            1,
            show_in_taskbar as usize,
        )
    } == 0
    {
        return Err(format!(
            "SetWindowSubclass failed: {}",
            io::Error::last_os_error()
        ));
    }
    let style = taskbar_style(style as u32, show_in_taskbar);
    unsafe { SetLastError(0) };
    if unsafe { SetWindowLongW(window, GWL_EXSTYLE, style as i32) } == 0
        && io::Error::last_os_error().raw_os_error() != Some(0)
    {
        return Err(format!(
            "SetWindowLongW failed: {}",
            io::Error::last_os_error()
        ));
    }
    if unsafe {
        SetWindowPos(
            window,
            ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        )
    } == 0
    {
        return Err(format!(
            "SetWindowPos failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

pub fn set_desktop_taskbar(show: bool) -> Result<(), String> {
    let window = desktop_window()?;
    let visible = unsafe { IsWindowVisible(window) } != 0;
    // The shell requires a hide/style/show cycle to refresh taskbar membership.
    if visible {
        unsafe { ShowWindow(window, SW_HIDE) };
    }
    let result = apply_taskbar_style(window, show);
    if visible {
        unsafe { ShowWindow(window, SW_SHOWNOACTIVATE) };
    }
    result
}

pub fn prepare_desktop_window(show_in_taskbar: bool) -> Result<DesktopAppearance, String> {
    let window = desktop_window()?;
    apply_taskbar_style(window, show_in_taskbar)?;
    let mut appearance = DesktopAppearance::Applied;
    for (attribute, value) in [
        (DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE),
        (DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND as u32),
    ] {
        let status = unsafe {
            DwmSetWindowAttribute(
                window,
                attribute as u32,
                ptr::from_ref(&value).cast(),
                size_of::<u32>() as u32,
            )
        };
        if status == E_INVALIDARG {
            // These fixed, correctly typed attributes require Windows 11 build 22000.
            appearance = DesktopAppearance::Unsupported;
        } else if status < 0 {
            return Err(format!(
                "DwmSetWindowAttribute({attribute}) failed: HRESULT 0x{status:08X}"
            ));
        }
    }
    Ok(appearance)
}
