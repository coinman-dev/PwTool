//! Windows of the program. The main window is eframe's root viewport;
//! every other form is a native window of its own (an immediate viewport),
//! like in the original. Modal dialogs disable their parent; a click on the
//! parent brings the dialog to the front. On Windows a dialog is an owned
//! window of its parent (stays above it, no taskbar button). On Wayland,
//! where a window can neither have a parent nor be raised, modal dialogs
//! are drawn inside the parent window instead (see `.ai/PLAN-egui.md` §9).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::{Context, Ui, Vec2, ViewportBuilder, ViewportClass, ViewportCommand, ViewportId};

/// What a window shows.
pub trait Content {
    fn ui(&self, ui: &mut Ui);
}

impl Content for crate::vcl::Form {
    fn ui(&self, ui: &mut Ui) {
        let fill = ui.visuals().panel_fill;
        ui.painter().rect_filled(ui.max_rect(), 0.0, fill);
        self.show(ui);
    }
}

/// A window other than the main window.
pub struct Window {
    pub id: ViewportId,
    pub title: RefCell<String>,
    pub content: Rc<dyn Content>,
    pub modal: bool,
    pub parent: ViewportId,
    pub size: Cell<Vec2>,
    pub min_size: Vec2,
    pub resizable: bool,
    /// Asked when the user closes the window; `false` keeps it open
    /// (`OnCloseQuery`).
    pub on_close: RefCell<Option<Box<dyn Fn() -> bool>>>,
    shown: Cell<bool>,
    #[cfg(windows)]
    owner_set: Cell<bool>,
}

impl Window {
    pub fn new(id: &str, title: &str, content: Rc<dyn Content>, parent: ViewportId, size: Vec2) -> Self {
        Window {
            id: ViewportId::from_hash_of(id),
            title: RefCell::new(title.to_string()),
            content,
            modal: false,
            parent,
            size: Cell::new(size),
            min_size: Vec2::ZERO,
            resizable: false,
            on_close: RefCell::new(None),
            shown: Cell::new(false),
            #[cfg(windows)]
            owner_set: Cell::new(false),
        }
    }

    pub fn modal(mut self, on: bool) -> Self {
        self.modal = on;
        self
    }

    pub fn resizable(mut self, on: bool, min: Vec2) -> Self {
        self.resizable = on;
        self.min_size = min;
        self
    }
}

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<Window>>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub fn set_context(ctx: &Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
}

fn repaint() {
    CTX.with(|c| {
        if let Some(c) = c.borrow().as_ref() {
            c.request_repaint();
        }
    });
}

/// Show a window (again).
pub fn open(w: Rc<Window>) {
    WINDOWS.with(|ws| {
        let mut ws = ws.borrow_mut();
        ws.retain(|x| x.id != w.id);
        ws.push(w);
    });
    repaint();
}

/// Close a window (without asking `on_close`).
pub fn close(id: ViewportId) {
    WINDOWS.with(|ws| ws.borrow_mut().retain(|x| x.id != id));
    repaint();
}

pub fn is_open(id: ViewportId) -> bool {
    WINDOWS.with(|ws| ws.borrow().iter().any(|x| x.id == id))
}

/// The modal window that blocks `id`, if any.
pub fn modal_child(id: ViewportId) -> Option<ViewportId> {
    WINDOWS.with(|ws| ws.borrow().iter().rev().find(|x| x.modal && x.parent == id).map(|x| x.id))
}

/// Whether any modal dialog is open (the tray menu is disabled then).
pub fn any_modal() -> bool {
    WINDOWS.with(|ws| ws.borrow().iter().any(|x| x.modal))
}

/// Modal dialogs are drawn inside their parent (Wayland).
pub fn embed_modal() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var("PWTOOL_X11").is_err()
}

/// Draw the content of window `id` (or the root), disabled while a modal
/// child is open, and then its child windows. Call from the root's `ui`.
pub fn show(ui: &mut Ui, id: ViewportId, content: &dyn Content) {
    let blocker = modal_child(id);
    let ctx = ui.ctx().clone();
    ui.add_enabled_ui(blocker.is_none(), |ui| content.ui(ui));
    if let Some(b) = blocker {
        if !embed_modal() && ctx.input(|i| i.pointer.any_pressed()) {
            ctx.send_viewport_cmd_to(b, ViewportCommand::Focus);
        }
        if embed_modal() {
            // dim the parent below the dialog drawn inside it
            let r = ui.max_rect();
            ui.painter().rect_filled(r, 0.0, egui::Color32::from_black_alpha(60));
        }
    }
    let children: Vec<Rc<Window>> = WINDOWS.with(|ws| ws.borrow().iter().filter(|w| w.parent == id).cloned().collect());
    for w in children {
        show_window(&ctx, &w);
    }
}

fn show_window(ctx: &Context, w: &Rc<Window>) {
    let embed = w.modal && embed_modal();
    let mut b = ViewportBuilder::default()
        .with_title(w.title.borrow().clone())
        .with_inner_size(w.size.get())
        .with_resizable(w.resizable)
        .with_maximize_button(w.resizable && !w.modal)
        .with_minimize_button(!w.modal);
    if w.resizable && w.min_size != Vec2::ZERO {
        b = b.with_min_inner_size(w.min_size);
    }
    if !w.resizable {
        b = b.with_min_inner_size(w.size.get()).with_max_inner_size(w.size.get());
    }
    if !w.shown.get() {
        // centre on the parent window
        if let Some(pr) = ctx.input_for(w.parent, |i| i.viewport().outer_rect) {
            let pos = pr.center() - w.size.get() / 2.0;
            b = b.with_position(pos.max(egui::pos2(0.0, 0.0)));
        }
    }
    if embed {
        ctx.set_embed_viewports(true);
    }
    let w2 = w.clone();
    let close_requested = ctx.show_viewport_immediate(w.id, b, move |ui, class| {
        let id = w2.id;
        if class == ViewportClass::EmbeddedWindow {
            ui.set_min_size(w2.size.get());
        }
        show(ui, id, &*w2.content);
        ui.input(|i| i.viewport().close_requested())
    });
    if embed {
        ctx.set_embed_viewports(false);
    }
    w.shown.set(true);
    #[cfg(windows)]
    if !w.owner_set.get() && !embed {
        if win32::set_owner(&w.title.borrow(), w.id, w.parent) {
            w.owner_set.set(true);
        }
    }
    if close_requested {
        let allow = w.on_close.borrow().as_ref().is_none_or(|f| f());
        if allow {
            close(w.id);
        } else {
            ctx.send_viewport_cmd_to(w.id, ViewportCommand::CancelClose);
        }
    }
}

/// Remember the native window of the main window (owner of dialogs on
/// Windows). Call from the root's `ui` with eframe's frame.
pub fn set_root_window(frame: &eframe::Frame) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = frame.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                win32::ROOT.with(|r| r.set(w.hwnd.get()));
            }
        }
    }
    #[cfg(not(windows))]
    let _ = frame;
}

/// Native window handle of the main window (Windows), 0 if unknown.
pub fn root_hwnd() -> isize {
    #[cfg(windows)]
    return win32::ROOT.with(|r| r.get());
    #[cfg(not(windows))]
    0
}

#[cfg(windows)]
mod win32 {
    use std::cell::RefCell;

    use egui::ViewportId;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWLP_HWNDPARENT, GetWindowTextW, GetWindowThreadProcessId, SetWindowLongPtrW,
    };

    thread_local! {
        pub static ROOT: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
        static KNOWN: RefCell<Vec<(ViewportId, isize)>> = const { RefCell::new(Vec::new()) };
    }

    fn own_windows() -> Vec<(HWND, String)> {
        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
            // SAFETY: lparam is the vector passed below.
            unsafe {
                let v = &mut *(lparam as *mut Vec<(HWND, String)>);
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, &mut pid);
                if pid == GetCurrentProcessId() {
                    let mut buf = [0u16; 512];
                    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
                    v.push((hwnd, String::from_utf16_lossy(&buf[..n.max(0) as usize])));
                }
            }
            1
        }
        let mut v: Vec<(HWND, String)> = Vec::new();
        // SAFETY: the callback uses the vector only during the call.
        unsafe { EnumWindows(Some(collect), &mut v as *mut _ as LPARAM) };
        v
    }

    fn hwnd_of(title: &str, exclude: &[isize]) -> Option<HWND> {
        own_windows().into_iter().find(|(h, t)| t == title && !exclude.contains(&(*h as isize))).map(|(h, _)| h)
    }

    /// Make the native window titled `title` an owned window of `parent`.
    pub fn set_owner(title: &str, id: ViewportId, parent: ViewportId) -> bool {
        KNOWN.with(|k| k.borrow_mut().retain(|(v, _)| *v != id));
        let known: Vec<isize> = KNOWN.with(|k| k.borrow().iter().map(|(_, h)| *h).collect());
        let parent_hwnd = if parent == ViewportId::ROOT {
            Some(ROOT.with(|r| r.get()) as HWND).filter(|h| !h.is_null())
        } else {
            KNOWN.with(|k| k.borrow().iter().find(|(id, _)| *id == parent).map(|(_, h)| *h as HWND))
        };
        let root = ROOT.with(|r| r.get());
        let mut exclude = known;
        exclude.push(root);
        let (Some(p), Some(dlg)) = (parent_hwnd, hwnd_of(title, &exclude)) else { return false };
        if p == dlg {
            return false;
        }
        // SAFETY: both are windows of this process.
        unsafe { SetWindowLongPtrW(dlg, GWLP_HWNDPARENT, p as isize) };
        KNOWN.with(|k| k.borrow_mut().push((id, dlg as isize)));
        true
    }
}
