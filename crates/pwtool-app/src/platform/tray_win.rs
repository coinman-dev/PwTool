//! System tray icon on Windows (`TTrayIcon` with `TrayMenu` of the main
//! form): `Shell_NotifyIconW` with a popup menu. The icon is owned by a
//! hidden window in its own thread; commands are sent to the GTK main loop
//! through a channel. Same interface as the StatusNotifierItem version
//! (`tray.rs`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW, GetSystemMetrics,
    GetWindowLongPtrW, HICON, HMENU, ICONINFO, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, SetMenuDefaultItem,
    SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE,
    WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};

/// Command from the tray to the main window.
#[derive(Debug, Clone)]
pub enum TrayCmd {
    /// Left click on the icon (`TrayIconDblClick`).
    Activate,
    /// Menu item (name of the `TMenuItem`).
    Menu(String),
    /// Profile entry of the Profile submenu.
    Profile(usize),
}

/// One entry of the tray menu.
#[derive(Debug, Clone)]
pub enum TrayEntry {
    Separator,
    Item { name: String, label: String, bold: bool },
    /// The Profile submenu: profile names and the entries after them.
    Profiles { label: String, rest: Vec<(String, String)> },
}

/// Icon image: straight (not premultiplied) BGRA pixels, top-down.
#[derive(Debug, Clone)]
pub struct Icon {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}

const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
const WM_TRAY_UPDATE: u32 = WM_APP + 2;
const WM_TRAY_BALLOON: u32 = WM_APP + 3;

/// State shared between the GTK thread and the tray thread.
struct Shared {
    title: String,
    icon: Vec<Icon>,
    entries: Vec<TrayEntry>,
    profiles: Vec<String>,
    enabled: bool,
    balloon: Option<(String, bool)>,
}

/// Data of the tray window (`GWLP_USERDATA`).
struct WindowData {
    shared: Arc<Mutex<Shared>>,
    tx: async_channel::Sender<TrayCmd>,
    online: Arc<AtomicBool>,
    hicon: HICON,
    taskbar_created: u32,
}

/// The tray window of the running icon, for [`notify`].
static CURRENT: Mutex<Option<(isize, Arc<Mutex<Shared>>)>> = Mutex::new(None);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copy a string into a fixed-size UTF-16 field (truncated, NUL-terminated).
fn copy_wide(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for u in s.encode_utf16() {
        if n + 1 >= dst.len() {
            break;
        }
        dst[n] = u;
        n += 1;
    }
    dst[n] = 0;
}

/// GTK mnemonic ("_File", "__") to a Windows menu label ("&File", "_").
fn menu_label(label: &str) -> String {
    let mut out = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '_' if chars.peek() == Some(&'_') => {
                chars.next();
                out.push('_');
            }
            '_' => out.push('&'),
            '&' => out.push_str("&&"),
            c => out.push(c),
        }
    }
    out
}

/// Create an icon handle from the image closest to the small icon size.
fn create_hicon(images: &[Icon]) -> HICON {
    // SAFETY: plain Win32 calls; the bitmaps are released after use.
    unsafe {
        let want = GetSystemMetrics(SM_CXSMICON).max(16);
        let Some(img) = images
            .iter()
            .filter(|i| i.width >= want)
            .min_by_key(|i| i.width)
            .or_else(|| images.iter().max_by_key(|i| i.width))
        else {
            return std::ptr::null_mut();
        };
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = img.width;
        bmi.bmiHeader.biHeight = -img.height;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color: HBITMAP = CreateDIBSection(std::ptr::null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let len = (img.width * img.height * 4) as usize;
        std::ptr::copy_nonoverlapping(img.data.as_ptr(), bits.cast::<u8>(), len.min(img.data.len()));
        let mask_bits = vec![0u8; (((img.width + 15) / 16 * 2) * img.height) as usize];
        let mask = CreateBitmap(img.width, img.height, 1, 1, mask_bits.as_ptr().cast());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: NOTIFYICONDATAW is plain data; zero is a valid initial value.
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

/// Add the icon (`NIM_ADD`) or update icon and tool tip (`NIM_MODIFY`).
fn apply(hwnd: HWND, data: &mut WindowData, add: bool) {
    let (title, icon) = {
        let s = data.shared.lock().unwrap();
        (s.title.clone(), s.icon.clone())
    };
    let new_icon = create_hicon(&icon);
    let mut nid = notify_data(hwnd);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY_CALLBACK;
    nid.hIcon = new_icon;
    copy_wide(&mut nid.szTip, &title);
    // SAFETY: valid NOTIFYICONDATAW for our window.
    let ok = unsafe { Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &nid) } != 0;
    if add {
        data.online.store(ok, Ordering::Relaxed);
    }
    if !data.hicon.is_null() {
        // SAFETY: icon created by us and no longer used by the shell.
        unsafe { DestroyIcon(data.hicon) };
    }
    data.hicon = new_icon;
}

fn show_balloon(hwnd: HWND, data: &WindowData) {
    let Some((text, error)) = data.shared.lock().unwrap().balloon.take() else { return };
    let mut nid = notify_data(hwnd);
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = if error { NIIF_ERROR } else { NIIF_INFO };
    copy_wide(&mut nid.szInfoTitle, crate::program::PROGRAM_NAME);
    copy_wide(&mut nid.szInfo, &text);
    // SAFETY: valid NOTIFYICONDATAW for our window.
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

/// Show the popup menu at the cursor and return the chosen command.
fn popup_menu(hwnd: HWND, data: &WindowData) -> Option<TrayCmd> {
    let (entries, profiles, enabled) = {
        let s = data.shared.lock().unwrap();
        (s.entries.clone(), s.profiles.clone(), s.enabled)
    };
    let mut cmds: Vec<TrayCmd> = Vec::new();
    let grayed = if enabled { 0 } else { MF_GRAYED };
    // SAFETY: menus are created, used and destroyed here; strings outlive the calls.
    unsafe {
        let menu: HMENU = CreatePopupMenu();
        let mut add = |m: HMENU, label: &str, cmd: TrayCmd, flags: u32| -> u32 {
            cmds.push(cmd);
            let id = cmds.len() as u32;
            let l = wide(&menu_label(label));
            AppendMenuW(m, MF_STRING | flags, id as usize, l.as_ptr());
            id
        };
        for e in &entries {
            match e {
                TrayEntry::Separator => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                }
                TrayEntry::Item { name, label, bold } => {
                    let id = add(menu, label, TrayCmd::Menu(name.clone()), grayed);
                    if *bold {
                        SetMenuDefaultItem(menu, id, 0);
                    }
                }
                TrayEntry::Profiles { label, rest } => {
                    let sub = CreatePopupMenu();
                    let shortcuts: Vec<char> = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars().collect();
                    for (i, p) in profiles.iter().enumerate() {
                        // "_0 name": mnemonic prefix, underscores of the name kept
                        let mut l = String::new();
                        if let Some(c) = shortcuts.get(i) {
                            l.push('_');
                            l.push(*c);
                            l.push(' ');
                        }
                        l.push_str(&p.replace('_', "__"));
                        add(sub, &l, TrayCmd::Profile(i), 0);
                    }
                    if !profiles.is_empty() {
                        AppendMenuW(sub, MF_SEPARATOR, 0, std::ptr::null());
                    }
                    for (name, l) in rest {
                        add(sub, l, TrayCmd::Menu(name.clone()), 0);
                    }
                    let l = wide(&menu_label(label));
                    AppendMenuW(menu, MF_POPUP | grayed, sub as usize, l.as_ptr());
                }
            }
        }
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // required so that the menu closes when clicking elsewhere
        SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, std::ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        if id > 0 { cmds.get(id as usize - 1).cloned() } else { None }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds the WindowData box set in the tray thread
    // (null before it is set and after WM_DESTROY).
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowData;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let data = &mut *ptr;
        match msg {
            WM_TRAY_CALLBACK => {
                match lparam as u32 {
                    WM_LBUTTONUP => {
                        let _ = data.tx.try_send(TrayCmd::Activate);
                    }
                    WM_RBUTTONUP => {
                        if let Some(cmd) = popup_menu(hwnd, data) {
                            let _ = data.tx.try_send(cmd);
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_TRAY_UPDATE => {
                apply(hwnd, data, false);
                0
            }
            WM_TRAY_BALLOON => {
                show_balloon(hwnd, data);
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                let nid = notify_data(hwnd);
                Shell_NotifyIconW(NIM_DELETE, &nid);
                if !data.hicon.is_null() {
                    DestroyIcon(data.hicon);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                PostQuitMessage(0);
                0
            }
            m if m == data.taskbar_created => {
                // Explorer was restarted: add the icon again
                apply(hwnd, data, true);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// A running tray icon.
pub struct TrayIcon {
    hwnd: isize,
    shared: Arc<Mutex<Shared>>,
    online: Arc<AtomicBool>,
}

impl TrayIcon {
    /// Show the tray icon. `None` if it cannot be created (not fatal: the
    /// program works without tray).
    pub fn spawn(
        title: &str,
        icon: Vec<Icon>,
        entries: Vec<TrayEntry>,
        profiles: Vec<String>,
        tx: async_channel::Sender<TrayCmd>,
    ) -> Option<TrayIcon> {
        let shared = Arc::new(Mutex::new(Shared {
            title: title.to_string(),
            icon,
            entries,
            profiles,
            enabled: true,
            balloon: None,
        }));
        let online = Arc::new(AtomicBool::new(false));
        let (rtx, rrx) = std::sync::mpsc::channel::<isize>();
        let (sh, on) = (shared.clone(), online.clone());
        let spawned = std::thread::Builder::new().name("pwtool-tray".into()).spawn(move || {
            // SAFETY: window creation and message loop of this thread.
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class = wide("PwToolTrayWindow");
                let mut wc: WNDCLASSW = std::mem::zeroed();
                wc.lpfnWndProc = Some(wnd_proc);
                wc.hInstance = instance;
                wc.lpszClassName = class.as_ptr();
                RegisterClassW(&wc); // fails harmlessly if already registered
                let hwnd = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if hwnd.is_null() {
                    let _ = rtx.send(0);
                    return;
                }
                let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
                let data = Box::new(WindowData { shared: sh, tx, online: on, hicon: std::ptr::null_mut(), taskbar_created });
                let ptr = Box::into_raw(data);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
                apply(hwnd, &mut *ptr, true);
                let _ = rtx.send(hwnd as isize);
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
        if let Err(e) = spawned {
            eprintln!("pwtool: tray icon not available: {e}");
            return None;
        }
        let hwnd = rrx.recv().unwrap_or(0);
        if hwnd == 0 {
            eprintln!("pwtool: tray icon not available: {}", std::io::Error::last_os_error());
            return None;
        }
        *CURRENT.lock().unwrap() = Some((hwnd, shared.clone()));
        Some(TrayIcon { hwnd, shared, online })
    }

    fn post(&self, msg: u32) {
        // SAFETY: posting to our tray window; ignored if it no longer exists.
        unsafe { PostMessageW(self.hwnd as HWND, msg, 0, 0) };
    }

    /// Whether the icon is shown in the notification area.
    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    pub fn set_title(&self, title: &str) {
        self.shared.lock().unwrap().title = title.to_string();
        self.post(WM_TRAY_UPDATE);
    }

    pub fn set_profiles(&self, profiles: Vec<String>) {
        self.shared.lock().unwrap().profiles = profiles;
    }

    pub fn set_icon(&self, icon: Vec<Icon>) {
        self.shared.lock().unwrap().icon = icon;
        self.post(WM_TRAY_UPDATE);
    }

    /// `TrayMenuPopup()`: items are disabled while a modal dialog is open.
    pub fn set_enabled(&self, enabled: bool) {
        self.shared.lock().unwrap().enabled = enabled;
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        let mut cur = CURRENT.lock().unwrap();
        if cur.as_ref().is_some_and(|(h, _)| *h == self.hwnd) {
            *cur = None;
        }
        self.post(WM_CLOSE);
    }
}

/// `ShowTrayInfo()`: balloon notification of the tray icon. Returns `false`
/// if no tray icon is shown.
pub fn notify(text: &str, error: bool) -> bool {
    let cur = CURRENT.lock().unwrap();
    let Some((hwnd, shared)) = cur.as_ref() else { return false };
    shared.lock().unwrap().balloon = Some((text.to_string(), error));
    // SAFETY: posting to our tray window.
    unsafe { PostMessageW(*hwnd as HWND, WM_TRAY_BALLOON, 0, 0) != 0 }
}

/// Icon for the tray from PNG images, one entry per size.
pub fn icon_from_pngs(images: &[&'static [u8]]) -> Vec<Icon> {
    images
        .iter()
        .filter_map(|b| crate::icons::decode_png(b))
        .map(|img| Icon {
            width: img.width as i32,
            height: img.height as i32,
            // BGRA, straight alpha
            data: img.pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect(),
        })
        .collect()
}
