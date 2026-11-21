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
        // forms with handlers handle their events right after drawing;
        // the others are polled with `take_events()`
        if self.has_handlers() {
            let _ = self.dispatch();
        }
    }
}

thread_local! {
    static INPUT_HOOK: RefCell<Option<Rc<dyn Fn(&Context)>>> = const { RefCell::new(None) };
    static INJECTED: RefCell<Vec<(ViewportId, egui::Event)>> = const { RefCell::new(Vec::new()) };
}

/// Automation: input events for window `id`, handled in its next frame as
/// if typed (the operating system delivers no keys to a window in the
/// background).
pub fn inject_input(id: ViewportId, events: Vec<egui::Event>) {
    let id = native_window(id);
    INJECTED.with(|q| q.borrow_mut().extend(events.into_iter().map(|e| (id, e))));
    ctx().request_repaint_of(id);
}

/// Function called for the input of every window in every frame (the
/// original collects entropy from all keyboard and mouse messages).
pub fn set_input_hook(f: impl Fn(&Context) + 'static) {
    INPUT_HOOK.with(|h| *h.borrow_mut() = Some(Rc::new(f)));
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
    /// The user can resize the window (`Cell`: set after it is made).
    pub resizable: Cell<bool>,
    pub decorated: bool,
    /// Asked when the user closes the window; `false` keeps it open
    /// (`OnCloseQuery`).
    pub on_close: RefCell<Option<Box<dyn Fn() -> bool>>>,
    shown: Cell<bool>,
    /// Size of the window after the last frame (resized by the user).
    pub last_size: Cell<Option<Vec2>>,
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
            resizable: Cell::new(false),
            decorated: true,
            on_close: RefCell::new(None),
            shown: Cell::new(false),
            last_size: Cell::new(None),
            #[cfg(windows)]
            owner_set: Cell::new(false),
        }
    }

    pub fn modal(mut self, on: bool) -> Self {
        self.modal = on;
        self
    }

    /// Draw `content` instead (a wrapper of the form, for example).
    pub fn with_content(mut self, content: Rc<dyn Content>) -> Self {
        self.content = content;
        self
    }

    pub fn resizable(mut self, on: bool, min: Vec2) -> Self {
        self.resizable.set(on);
        self.min_size = min;
        self
    }
}

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<Window>>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// The egui context of the program.
pub fn ctx() -> Context {
    CTX.with(|c| c.borrow().clone()).expect("windows::set_context not called")
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

/// Size of an open window after the last frame.
pub fn last_size(id: ViewportId) -> Option<Vec2> {
    WINDOWS.with(|ws| ws.borrow().iter().find(|x| x.id == id).and_then(|x| x.last_size.get()))
}

/// Bring an open window to the front.
pub fn focus(id: ViewportId) {
    send_cmd(id, ViewportCommand::Focus);
}

/// Whether window `id` is drawn inside its parent (no native window).
fn is_embedded(id: ViewportId) -> bool {
    embed_modal() && WINDOWS.with(|ws| ws.borrow().iter().any(|x| x.id == id && x.modal))
}

/// The native window that shows window `id`: the window itself or, for a
/// dialog drawn inside its parent, the parent's native window.
fn native_window(id: ViewportId) -> ViewportId {
    let mut id = id;
    while is_embedded(id) {
        match WINDOWS.with(|ws| ws.borrow().iter().find(|x| x.id == id).map(|x| x.parent)) {
            Some(p) => id = p,
            None => break,
        }
    }
    id
}

/// Send a command to the native window of window `id` if it is open.
/// egui creates a native window for every unknown viewport a command is
/// sent to (or whose input is read): a window that is closed or drawn
/// inside its parent must not get one.
pub fn send_cmd(id: ViewportId, cmd: ViewportCommand) {
    if id == ViewportId::ROOT || is_open(id) {
        ctx().send_viewport_cmd_to(native_window(id), cmd);
    }
}

/// Show a window that may be open already: an open one is only brought
/// to the front (`Show()` of a form that is kept).
pub fn present(w: Rc<Window>) {
    if is_open(w.id) {
        focus(w.id);
    } else {
        open(w);
    }
}

/// The modal window that blocks `id`, if any.
pub fn modal_child(id: ViewportId) -> Option<ViewportId> {
    WINDOWS.with(|ws| ws.borrow().iter().rev().find(|x| x.modal && x.parent == id).map(|x| x.id))
}

/// Whether any modal dialog is open (the tray menu is disabled then).
pub fn any_modal() -> bool {
    WINDOWS.with(|ws| ws.borrow().iter().any(|x| x.modal))
}

/// The windows are Wayland windows (winit prefers Wayland when it is there).
pub fn is_wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var("PWTOOL_X11").is_err()
}

/// Modal dialogs are drawn inside their parent (Wayland).
pub fn embed_modal() -> bool {
    is_wayland()
}

/// Draw the content of window `id` (or the root), disabled while a modal
/// child is open, and then its child windows. Call from the root's `ui`.
pub fn show(ui: &mut Ui, id: ViewportId, content: &dyn Content) {
    let blocker = modal_child(id);
    let ctx = ui.ctx().clone();
    if let Some(h) = INPUT_HOOK.with(|h| h.borrow().clone()) {
        h(&ctx);
    }
    if ctx.viewport_id() == id {
        let mine: Vec<egui::Event> = INJECTED.with(|q| {
            let mut q = q.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q).into_iter().partition(|(v, _)| *v == id);
            *q = rest;
            mine.into_iter().map(|(_, e)| e).collect()
        });
        if !mine.is_empty() {
            ctx.input_mut(|i| i.events.extend(mine));
        }
    }
    // screen readers: the window is named by its title (not for a dialog
    // drawn inside its parent, which would rename the parent)
    if ctx.viewport_id() == id {
        if let Some(title) = ctx.input(|i| i.viewport().title.clone()) {
            ctx.accesskit_node_builder(egui::accesskit_root_id(), |b| b.set_label(title));
        }
    }
    ui.add_enabled_ui(blocker.is_none(), |ui| content.ui(ui));
    crate::ui::automate::save_screenshots(&ctx);
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

thread_local! {
    /// Size of the frame and title bar of a dialog drawn inside its parent
    /// (measured; `None` until a dialog has been drawn).
    static EMBED_FRAME: Cell<Option<Vec2>> = const { Cell::new(None) };
    static EMBED_CONTENT: Cell<Vec2> = const { Cell::new(Vec2::ZERO) };
}

/// Free space around a dialog drawn inside its parent.
const EMBED_MARGIN: f32 = 4.0;

/// Largest content size of a dialog drawn inside the current window.
fn embed_room(ctx: &Context) -> Vec2 {
    let frame = EMBED_FRAME.with(|f| f.get()).unwrap_or_else(|| {
        // until measured: margins of the frame and a title bar
        let style = ctx.global_style();
        let title = style.text_styles.get(&egui::TextStyle::Heading).map_or(18.0, |f| f.size);
        style.spacing.window_margin.sum() + egui::vec2(2.0, 2.0 + title * 1.5 + 2.0 * style.spacing.item_spacing.y)
    });
    (ctx.content_rect().size() - frame - Vec2::splat(2.0 * EMBED_MARGIN)).max(Vec2::splat(50.0)).floor()
}

/// Measure the frame of the dialog `id` drawn inside the current window
/// (drawn once more if it differs from the estimate).
fn measure_embed_frame(ctx: &Context, id: ViewportId) {
    let Some(r) = ctx.memory(|m| m.area_rect(egui::Id::new(id))) else { return };
    let frame = (r.size() - EMBED_CONTENT.with(|c| c.get())).max(Vec2::ZERO).ceil();
    let old = EMBED_FRAME.with(|f| f.replace(Some(frame)));
    if old.is_none_or(|o| (o - frame).abs().max_elem() > 0.5) {
        ctx.request_repaint();
    }
}

fn show_window(ctx: &Context, w: &Rc<Window>) {
    let embed = w.modal && embed_modal();
    let mut size = w.size.get();
    let resizable = w.resizable.get();
    let mut min = if resizable { w.min_size } else { size };
    let mut max = (!resizable).then_some(size);
    if embed {
        // a dialog drawn inside its parent is no larger than the parent:
        // its controls follow their anchors to the smaller size
        let room = embed_room(ctx);
        size = size.min(room);
        min = min.min(room);
        max = Some(max.unwrap_or(room).min(room));
    }
    let mut b = ViewportBuilder::default()
        .with_title(w.title.borrow().clone())
        .with_inner_size(size)
        .with_resizable(resizable)
        .with_maximize_button(resizable && !w.modal)
        .with_minimize_button(!w.modal)
        .with_decorations(w.decorated);
    if min != Vec2::ZERO {
        b = b.with_min_inner_size(min);
    }
    if let Some(max) = max {
        b = b.with_max_inner_size(max);
    }
    if !w.shown.get() && !embed {
        // centre on the parent window
        if let Some(pr) = ctx.input_for(native_window(w.parent), |i| i.viewport().outer_rect) {
            let pos = pr.center() - w.size.get() / 2.0;
            b = b.with_position(pos.max(egui::pos2(0.0, 0.0)));
        }
    }
    if embed {
        ctx.set_embed_viewports(true);
    }
    let w2 = w.clone();
    let close_requested = ctx.show_viewport_immediate(w.id, b, move |ui, class| {
        if class == ViewportClass::EmbeddedWindow {
            ui.set_min_size(size);
            show(ui, w2.id, &*w2.content);
            EMBED_CONTENT.with(|c| c.set(ui.min_rect().size()));
            // the viewport input is the parent's: no close button of its own
            return false;
        }
        show(ui, w2.id, &*w2.content);
        ui.input(|i| i.viewport().close_requested())
    });
    if embed {
        ctx.set_embed_viewports(false);
        measure_embed_frame(ctx, w.id);
    }
    w.shown.set(true);
    if embed {
        w.last_size.set(Some(EMBED_CONTENT.with(|c| c.get())));
    } else if let Some(r) = ctx.input_for(w.id, |i| i.viewport().inner_rect) {
        w.last_size.set(Some(r.size()));
    }
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

/// Titles of the open windows (test automation).
pub fn open_ids() -> Vec<String> {
    WINDOWS.with(|ws| ws.borrow().iter().map(|w| w.title.borrow().clone()).collect())
}
