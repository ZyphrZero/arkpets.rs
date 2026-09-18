use std::error::Error;
use x11rb::{
    connection::Connection,
    errors::ReplyError,
    protocol::{
        xproto::{
            Atom, AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, GetPropertyReply,
            MapState, PropMode, Window,
        },
        ErrorKind,
    },
    wrapper::ConnectionExt as _,
};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_CLIENT_LIST,
        _NET_SUPPORTED,
        _NET_WM_PID,
        _NET_WM_NAME,
        UTF8_STRING,
        _NET_WM_STATE,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NORMAL,
        _NET_WM_WINDOW_TYPE_UTILITY,
    }
}

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn property(
    conn: &impl Connection,
    window: Window,
    atom: Atom,
    kind: Atom,
    length: u32,
) -> Result<Option<GetPropertyReply>> {
    match conn
        .get_property(false, window, atom, kind, 0, length)?
        .reply()
    {
        Ok(reply) => Ok(Some(reply)),
        // Other applications may destroy a window while we inspect the client list.
        Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Window => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn desktop_window(conn: &impl Connection, root: Window, atoms: &Atoms) -> Result<Window> {
    // Unmapped windows are root children; managed windows can be reparented by the WM.
    let mut candidates = conn.query_tree(root)?.reply()?.children;
    if let Some(clients) = property(
        conn,
        root,
        atoms._NET_CLIENT_LIST,
        AtomEnum::WINDOW.into(),
        65536,
    )? {
        if let Some(values) = clients.value32() {
            candidates.extend(values);
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    for window in candidates {
        let Some(pid) = property(
            conn,
            window,
            atoms._NET_WM_PID,
            AtomEnum::CARDINAL.into(),
            1,
        )?
        else {
            continue;
        };
        if pid.value32().and_then(|mut values| values.next()) != Some(std::process::id()) {
            continue;
        }
        let Some(name) = property(conn, window, atoms._NET_WM_NAME, atoms.UTF8_STRING, 128)? else {
            continue;
        };
        if name.value == "ArkPets - \u{684c}\u{5ba0}".as_bytes() {
            return Ok(window);
        }
    }
    Err("Desktop pet X11 window was not found for the current process".into())
}

pub fn set_visible(show: bool) -> Result<()> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let atoms = Atoms::new(&conn)?.reply()?;
    let supported = property(
        &conn,
        root,
        atoms._NET_SUPPORTED,
        AtomEnum::ATOM.into(),
        65536,
    )?
    .ok_or("X11 root window disappeared")?;
    let supported: Vec<_> = supported.value32().into_iter().flatten().collect();
    let skip = [
        atoms._NET_WM_STATE_SKIP_TASKBAR,
        atoms._NET_WM_STATE_SKIP_PAGER,
    ];
    if !skip.iter().all(|atom| supported.contains(atom)) {
        return Err(
            "The window manager does not support EWMH taskbar/pager visibility hints".into(),
        );
    }
    let window = desktop_window(&conn, root, &atoms)?;
    let kind = if show {
        atoms._NET_WM_WINDOW_TYPE_NORMAL
    } else {
        atoms._NET_WM_WINDOW_TYPE_UTILITY
    };
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms._NET_WM_WINDOW_TYPE,
        AtomEnum::ATOM,
        &[kind],
    )?
    .check()?;
    if conn.get_window_attributes(window)?.reply()?.map_state == MapState::UNMAPPED {
        let current = property(
            &conn,
            window,
            atoms._NET_WM_STATE,
            AtomEnum::ATOM.into(),
            65536,
        )?
        .ok_or("Desktop pet X11 window disappeared")?;
        let mut states: Vec<_> = current
            .value32()
            .into_iter()
            .flatten()
            .filter(|atom| !skip.contains(atom))
            .collect();
        if !show {
            states.extend(skip);
        }
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_STATE,
            AtomEnum::ATOM,
            &states,
        )?
        .check()?;
    } else {
        // Managed windows request a state change from the WM; never unmap or minimize.
        let event = ClientMessageEvent::new(
            32,
            window,
            atoms._NET_WM_STATE,
            [u32::from(!show), skip[0], skip[1], 1, 0],
        );
        conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?
        .check()?;
    }
    conn.flush()?;
    Ok(())
}
