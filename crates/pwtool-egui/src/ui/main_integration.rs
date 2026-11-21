//! Tray icon, minimizing to the tray, notifications, global hot keys and
//! autotype of the main window (child module of `main_window`).

use pwtool_app::app_config::{HotKeyList, hot_key_action};
use pwtool_app::autotype;
use pwtool_app::platform::tray::{TrayCmd, TrayEntry, TrayIcon, icon_from_pngs};

use super::*;

const AUTOTYPE_TOKEN_KEY: &str = "AutotypeRestoreToken";

/// State of the tray icon and the hidden main window.
pub(super) struct Integration {
    tray: RefCell<Option<TrayIcon>>,
    tx: async_channel::Sender<TrayCmd>,
    rx: async_channel::Receiver<TrayCmd>,
    /// `APPSTATE_HIDDEN`: main window hidden in the tray.
    pub hidden: Cell<bool>,
    tray_title: RefCell<String>,
    tray_icon_name: RefCell<String>,
    menu_disabled: Cell<bool>,
    last_spawn_failure: Cell<Option<std::time::Instant>>,
    pub hot_keys_active: RefCell<HotKeyList>,
    pub autotype_active: Cell<bool>,
    autotype_abort: RefCell<Option<Rc<Cell<bool>>>>,
    was_minimized: Cell<bool>,
    pub focused: Cell<bool>,
}

impl Integration {
    pub fn new() -> Self {
        let (tx, rx) = async_channel::unbounded();
        Integration {
            tray: RefCell::new(None),
            tx,
            rx,
            hidden: Cell::new(false),
            tray_title: RefCell::new(String::new()),
            tray_icon_name: RefCell::new(String::new()),
            menu_disabled: Cell::new(false),
            last_spawn_failure: Cell::new(None),
            hot_keys_active: RefCell::new(HotKeyList::new()),
            autotype_active: Cell::new(false),
            autotype_abort: RefCell::new(None),
            was_minimized: Cell::new(false),
            focused: Cell::new(true),
        }
    }
}

/// VCL caption (`&File`) → mnemonic of the tray menus (`_File`).
fn tray_label(caption: &str) -> String {
    let mut out = String::new();
    let mut it = caption.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '&' if it.peek() == Some(&'&') => {
                it.next();
                out.push('&');
            }
            '&' => out.push('_'),
            '_' => out.push_str("__"),
            c => out.push(c),
        }
    }
    out
}

impl MainWindow {
    pub(super) fn setup_integration(self: &Rc<Self>) {
        let rx = self.integ.rx.clone();
        let w = self.weak();
        spawn(async move {
            while let Ok(cmd) = rx.recv().await {
                let Some(w) = w.upgrade() else { break };
                match cmd {
                    TrayCmd::Activate => w.restore_from_tray(),
                    TrayCmd::Profile(i) => {
                        let Some(name) = w.core.borrow().profiles.get(i).map(|p| p.profile_name.clone()) else { continue };
                        w.load_profile(i);
                        w.show_tray_info(&trl_format("Profile \"%1\" activated.", &[&name]), false);
                    }
                    TrayCmd::Menu(name) => {
                        if name == "TrayMenu_Restore" {
                            w.restore_from_tray();
                        } else {
                            w.form.push_event(crate::vcl::Event::Menu(name));
                        }
                    }
                }
            }
        });
        self.menu("TrayMenu_Exit", |w| {
            w.restore_from_tray();
            windows::ctx().send_viewport_cmd(ViewportCommand::Close);
        });
        self.menu("TrayMenu_ClearClip", |w| w.clear_clip_btn_click());
        self.menu("TrayMenu_GenPassw", |w| w.generate_passw(GenDest::Clipboard));
        self.menu("TrayMenu_GenAndShowPassw", |w| w.generate_passw(GenDest::MsgBox));
        self.menu("TrayMenu_GenAndAutoTypePassw", |w| w.generate_passw(GenDest::Autotype));
        self.menu("TrayMenu_ResetWindowPos", |_| {
            let ctx = windows::ctx();
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(40.0, 40.0)));
        });
        let keys = self.core.borrow().config.hot_keys.clone();
        self.activate_hot_keys(&keys);
        self.update_tray_icon();
    }

    /// Menu entries of `TrayMenu` (order of the original form).
    fn tray_entries(&self) -> Vec<TrayEntry> {
        let mut v = Vec::new();
        let caption = |o: &pwtool_app::form::FormObj| tray_label(&trl(o.str("Caption").unwrap_or("")));
        let Some(menu) = self.form.obj.find("TrayMenu") else { return v };
        for it in menu.children.iter().filter(|c| c.class == "TMenuItem") {
            if it.str("Caption") == Some("-") {
                v.push(TrayEntry::Separator);
                continue;
            }
            match it.name.as_str() {
                "TrayMenu_Profile" => {
                    let rest = it
                        .children
                        .iter()
                        .filter(|c| c.class == "TMenuItem" && c.str("Caption") != Some("-"))
                        .map(|c| (c.name.clone(), caption(c)))
                        .collect();
                    v.push(TrayEntry::Profiles { label: caption(it), rest });
                }
                "TrayMenu_About" => v.push(TrayEntry::Item {
                    name: it.name.clone(),
                    label: tray_label(&trl_format("About %1...", &[PROGRAM_NAME])),
                    bold: false,
                }),
                "TrayMenu_Restore" => v.push(TrayEntry::Item {
                    name: it.name.clone(),
                    label: trl_format("Restore %1", &[PROGRAM_NAME]),
                    bold: true,
                }),
                _ => v.push(TrayEntry::Item { name: it.name.clone(), label: caption(it), bold: false }),
            }
        }
        v
    }

    /// Show/hide the tray icon according to the configuration and update
    /// icon, title and profiles.
    pub(super) fn update_tray_icon(&self) {
        let (visible, icon_name) = {
            let core = self.core.borrow();
            (core.config.show_sys_tray_icon_const || self.integ.hidden.get(), core.config.app_icon_name.clone())
        };
        let title = self.title.borrow().clone();
        let profiles: Vec<String> = self.core.borrow().profiles.iter().map(|p| p.profile_name.clone()).collect();
        let mut icon = self.integ.tray.borrow_mut();
        if !visible {
            icon.take();
            return;
        }
        match icon.as_ref() {
            None => {
                if self.integ.last_spawn_failure.get().is_some_and(|t| t.elapsed().as_secs() < 60) {
                    return;
                }
                *icon = TrayIcon::spawn(
                    &title,
                    icon_from_pngs(&pwtool_app::icons::app_icon_pngs(&icon_name)),
                    self.tray_entries(),
                    profiles,
                    self.integ.tx.clone(),
                );
                if icon.is_none() {
                    self.integ.last_spawn_failure.set(Some(std::time::Instant::now()));
                }
                *self.integ.tray_title.borrow_mut() = title;
                *self.integ.tray_icon_name.borrow_mut() = icon_name;
            }
            Some(t) => {
                if *self.integ.tray_title.borrow() != title {
                    t.set_title(&title);
                    *self.integ.tray_title.borrow_mut() = title;
                }
                if *self.integ.tray_icon_name.borrow() != icon_name {
                    t.set_icon(icon_from_pngs(&pwtool_app::icons::app_icon_pngs(&icon_name)));
                    *self.integ.tray_icon_name.borrow_mut() = icon_name;
                }
                t.set_profiles(profiles);
            }
        }
    }

    /// Every second: tray title, tray menu disabled while a modal dialog
    /// is open or a generation runs (`TrayMenuPopup()`).
    pub(super) fn tray_timer(&self) {
        if let Some(t) = self.integ.tray.borrow().as_ref() {
            let busy = self.generating.get() || windows::any_modal();
            if busy != self.integ.menu_disabled.get() {
                self.integ.menu_disabled.set(busy);
                t.set_enabled(!busy);
            }
            let title = self.title.borrow().clone();
            if *self.integ.tray_title.borrow() != title {
                t.set_title(&title);
                *self.integ.tray_title.borrow_mut() = title;
            }
        }
    }

    fn tray_online(&self) -> bool {
        self.integ.tray.borrow().as_ref().is_some_and(|t| t.is_online())
    }

    /// Called every frame with the state of the main window.
    pub fn watch_window(self: &Rc<Self>, ctx: &egui::Context) {
        let (minimized, focused) = ctx.input(|i| (i.viewport().minimized.unwrap_or(false), i.viewport().focused.unwrap_or(false)));
        self.integ.focused.set(focused);
        if minimized && !self.integ.was_minimized.get() {
            self.app_minimize();
        }
        self.integ.was_minimized.set(minimized);
    }

    /// `AppMinimize()`: hide the window in the tray. Wayland cannot hide
    /// windows; there the window stays minimized.
    fn app_minimize(&self) {
        if self.core.borrow().config.minimize_to_sys_tray && !self.integ.hidden.get() && !self.integ.autotype_active.get() {
            self.integ.hidden.set(true);
            self.update_tray_icon();
            if self.tray_online() && !windows::embed_modal() {
                windows::ctx().send_viewport_cmd(ViewportCommand::Visible(false));
            } else {
                self.integ.hidden.set(false);
            }
        }
    }

    /// Show the main window again (`AppRestore()`).
    pub fn restore_from_tray(&self) {
        self.integ.hidden.set(false);
        let ctx = windows::ctx();
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.update_tray_icon();
    }

    /// Hide the main window at start-up (`--silent`).
    pub fn start_hidden(self: &Rc<Self>) {
        self.integ.hidden.set(true);
        self.update_tray_icon();
        windows::ctx().send_viewport_cmd(ViewportCommand::Visible(false));
        // no tray host: the window must stay reachable
        let w = self.weak();
        spawn(async move {
            pwtool_app::rt::sleep(Duration::from_millis(1500)).await;
            if let Some(w) = w.upgrade() {
                if w.integ.hidden.get() && !w.tray_online() {
                    w.restore_from_tray();
                }
            }
        });
    }

    pub fn hot_key_count(&self) -> usize {
        self.integ.hot_keys_active.borrow().len()
    }

    pub fn is_hidden(&self) -> bool {
        self.integ.hidden.get()
    }

    /// `ShowTrayInfo()`: notification while hidden, message box otherwise.
    pub fn show_tray_info(&self, text: &str, warning: bool) {
        if !self.integ.hidden.get() {
            msg(ROOT, text, if warning { MsgIcon::Warning } else { MsgIcon::Info });
            return;
        }
        let text = text.to_string();
        spawn(async move {
            if !pwtool_app::platform::notify::notify(&text, warning).await {
                eprintln!("pwtool: {text}");
            }
        });
    }

    // ----------------------------------------------------------------------
    // hot keys

    /// `ActivateHotKeys()`.
    pub fn activate_hot_keys(self: &Rc<Self>, keys: &HotKeyList) {
        *self.integ.hot_keys_active.borrow_mut() = keys.clone();
        if keys.is_empty() {
            pwtool_app::platform::hotkeys::deactivate();
            return;
        }
        let show = trl("Show/restore main window");
        let actions: Vec<String> = pwtool_app::form::try_load_form("Configuration")
            .and_then(|f| f.find("HotKeyActionsList").map(|o| o.strings("Items.Strings")))
            .unwrap_or_default();
        let list: Vec<(u16, String)> = keys
            .iter()
            .map(|(k, e)| {
                let mut d = String::new();
                if e.show_main_win {
                    d.push_str(&show);
                }
                if e.action != hot_key_action::NONE {
                    if !d.is_empty() {
                        d.push_str(", ");
                    }
                    d.push_str(&trl(actions.get(e.action as usize).map(|s| s.as_str()).unwrap_or("")));
                }
                (*k, d)
            })
            .collect();
        let w = self.weak();
        let key_names: Vec<String> = list.iter().map(|(k, _)| pwtool_app::shortcut::shortcut_to_text(*k)).collect();
        spawn(async move {
            let w2 = w.clone();
            let cb: Rc<dyn Fn(u16)> = Rc::new(move |code| {
                if let Some(w) = w2.upgrade() {
                    w.on_hot_key(code);
                }
            });
            if let Err(e) = pwtool_app::platform::hotkeys::activate(None, list, cb).await {
                eprintln!("pwtool: global hot keys: {e}");
                if w.upgrade().is_some_and(|w| !w.integ.hidden.get()) {
                    msg(ROOT, &format!("{}\n{}", trl("Could not register the following hot keys:"), key_names.join("\n")), MsgIcon::Warning);
                }
            }
        });
    }

    /// `OnHotKey()`.
    fn on_hot_key(self: &Rc<Self>, code: u16) {
        let Some(hke) = self.integ.hot_keys_active.borrow().get(&code).copied() else { return };
        if self.generating.get() || windows::any_modal() {
            return;
        }
        let was_hidden = self.integ.hidden.get();
        let show_main = |force: bool| {
            if force || hke.show_main_win {
                self.restore_from_tray();
            }
        };
        let dest = match hke.action {
            hot_key_action::SINGLE_PASSW => {
                show_main(false);
                Some(GenDest::GuiSingle)
            }
            hot_key_action::PASSW_LIST => {
                show_main(false);
                Some(GenDest::GuiList)
            }
            hot_key_action::PASSW_CLIPBOARD => {
                show_main(false);
                Some(GenDest::Clipboard)
            }
            hot_key_action::PASSW_MSGBOX => {
                let _ = was_hidden;
                show_main(true);
                Some(GenDest::MsgBox)
            }
            hot_key_action::PASSW_AUTOTYPE => {
                show_main(false);
                Some(GenDest::Autotype)
            }
            // password manager and MP generator: stage 3c/4
            _ => {
                show_main(true);
                None
            }
        };
        if let Some(d) = dest {
            self.generate_passw(d);
        }
    }

    // ----------------------------------------------------------------------
    // autotype

    pub fn abort_autotype(&self) {
        if let Some(a) = self.integ.autotype_abort.borrow_mut().take() {
            a.set(true);
        }
    }

    pub fn is_autotype_active(&self) -> bool {
        self.integ.autotype_active.get()
    }

    /// Type key strokes into the window that gets the focus next
    /// (`TSendKeysThread`).
    pub fn send_keys(self: &Rc<Self>, strokes: Vec<autotype::KeyStroke>, minimize: bool) {
        if strokes.is_empty() {
            return;
        }
        self.abort_autotype();
        let abort = Rc::new(Cell::new(false));
        *self.integ.autotype_abort.borrow_mut() = Some(abort.clone());
        let w = self.clone();
        spawn(async move {
            struct ActiveGuard(Rc<MainWindow>, Rc<Cell<bool>>);
            impl Drop for ActiveGuard {
                fn drop(&mut self) {
                    self.0.integ.autotype_active.set(false);
                    let mut slot = self.0.integ.autotype_abort.borrow_mut();
                    if slot.as_ref().is_some_and(|cur| Rc::ptr_eq(cur, &self.1)) {
                        slot.take();
                    }
                }
            }
            let token = w.core.borrow().ini.read_string("Main", AUTOTYPE_TOKEN_KEY, "");
            // permission first: the desktop may show a dialog (Linux)
            match autotype::prepare(None, &token).await {
                Ok(Some(t)) => w.core.borrow_mut().ini.write_string("Main", AUTOTYPE_TOKEN_KEY, &t),
                Ok(None) => {}
                Err(e) => {
                    msg(ROOT, &format!("{}\n\n{e}", trl("Autotype is not available (RemoteDesktop portal).")), MsgIcon::Error);
                    return;
                }
            }
            if abort.get() {
                return;
            }
            w.integ.autotype_active.set(true);
            let _guard = ActiveGuard(w.clone(), abort.clone());
            if minimize {
                windows::ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
            }
            // wait until another window is active (also after minimizing)
            let start = std::time::Instant::now();
            pwtool_app::rt::sleep(Duration::from_millis(100)).await;
            while w.integ.focused.get() {
                if abort.get() || start.elapsed().as_secs() >= 10 {
                    return;
                }
                pwtool_app::rt::sleep(Duration::from_millis(100)).await;
            }
            if let Err(e) = autotype::send(None, "", strokes, abort).await {
                w.show_tray_info(&format!("{}\n\n{e}", trl("Autotype failed.")), true);
            }
        });
    }
}
