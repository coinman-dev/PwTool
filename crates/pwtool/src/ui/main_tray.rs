//! Tray icon and minimizing to the tray (`TrayIcon`, `AppMinimize()`,
//! `AppRestore()`, `TrayMenu*`). Child module of `main_window`.

use super::*;
use crate::ui::tray::{TrayCmd, TrayEntry, TrayIcon, icon_from_pngs};

/// PNG images (16, 32, 48 pixels) of the icon for
/// `Configuration::AppIconName`.
fn app_icon_pngs(name: &str) -> [&'static [u8]; 3] {
    macro_rules! icon {
        ($n:literal) => {
            [
                include_bytes!(concat!("../../../../assets/appicons/", $n, "_16.png")),
                include_bytes!(concat!("../../../../assets/appicons/", $n, "_32.png")),
                include_bytes!(concat!("../../../../assets/appicons/", $n, "_48.png")),
            ]
        };
    }
    match name {
        "Blue" => icon!("pwtool_icon_blue"),
        "Red" => icon!("pwtool_icon_red"),
        "Orange" => icon!("pwtool_icon_orange"),
        "Green" => icon!("pwtool_icon_green"),
        "Colors" => icon!("pwtool_icon_colored"),
        _ => icon!("pwtool_default"),
    }
}

pub(super) struct TrayState {
    icon: RefCell<Option<TrayIcon>>,
    tx: async_channel::Sender<TrayCmd>,
    rx: async_channel::Receiver<TrayCmd>,
    /// `APPSTATE_HIDDEN`: main window hidden (minimized to tray or started
    /// with `--silent`).
    pub hidden: Cell<bool>,
    title: RefCell<String>,
    icon_name: RefCell<String>,
    last_spawn_failure: Cell<Option<std::time::Instant>>,
    menu_disabled: Cell<bool>,
}

impl TrayState {
    pub fn new() -> Self {
        let (tx, rx) = async_channel::unbounded();
        TrayState {
            icon: RefCell::new(None),
            tx,
            rx,
            hidden: Cell::new(false),
            title: RefCell::new(String::new()),
            icon_name: RefCell::new(String::new()),
            last_spawn_failure: Cell::new(None),
            menu_disabled: Cell::new(false),
        }
    }
}

impl MainWindow {
    /// Menu entries of `TrayMenu` (order of the original form).
    fn tray_entries(&self) -> Vec<TrayEntry> {
        let mut v = Vec::new();
        let caption = |o: &crate::ui::form::FormObj| {
            crate::ui::builder::mnemonic(&crate::ui::builder::tr_caption(o.str("Caption").unwrap_or("")))
        };
        let Some(menu) = self.b.form.find("TrayMenu") else { return v };
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
                "TrayMenu_About" => {
                    let l = crate::ui::builder::mnemonic(&trl_format("About %1...", &[PROGRAM_NAME]));
                    v.push(TrayEntry::Item { name: it.name.clone(), label: l, bold: false });
                }
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

    pub(super) fn setup_tray(self: &Rc<Self>) {
        let rx = self.tray.rx.clone();
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(cmd) = rx.recv().await {
                let Some(w) = w.upgrade() else { break };
                match cmd {
                    TrayCmd::Activate => w.tray_activate(),
                    TrayCmd::Profile(i) => w.tray_profile(i),
                    TrayCmd::Menu(name) => {
                        if name == "TrayMenu_Restore" {
                            w.restore_from_tray();
                        } else if w.b.has_action(&name) {
                            gtk::prelude::ActionGroupExt::activate_action(&w.b.actions, &name, None);
                        }
                    }
                }
            }
        });
        // minimizing (`AppMinimize()`): KWin reports it (Wayland does not
        // tell applications); GDK's state is used where available (X11)
        if let Some(app) = self.window.application() {
            let w = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let w2 = w.clone();
                crate::ui::active_window::watch_minimized(&app, move |caption, minimized| {
                    if let Some(w) = w2.upgrade() {
                        let title = w.window.title().map(|t| t.to_string()).unwrap_or_default();
                        if minimized && caption == title {
                            w.app_minimize();
                        } else if minimized {
                            // the password manager window (`LockMinimize`)
                            let mng = w.passw_mng.borrow().clone();
                            if let Some(m) = mng {
                                let t = m.win.title().map(|t| t.to_string()).unwrap_or_default();
                                if t == caption && w.core.borrow().config.database.lock_minimize {
                                    m.lock_if_open();
                                }
                            }
                        }
                    }
                })
                .await;
            });
        }
        let w = Rc::downgrade(self);
        self.window.connect_realize(move |win| {
            let Some(surface) = win.surface() else { return };
            let Ok(top) = surface.downcast::<gdk::Toplevel>() else { return };
            let w = w.clone();
            top.connect_state_notify(move |t: &gdk::Toplevel| {
                if let Some(w) = w.upgrade() {
                    if t.state().contains(gdk::ToplevelState::MINIMIZED) {
                        w.app_minimize();
                    }
                }
            });
        });
        self.update_tray_icon();
    }

    /// `AppMinimize()`.
    fn app_minimize(self: &Rc<Self>) {
        // lock the database (`LockMinimize`)
        if self.core.borrow().config.database.lock_minimize {
            let mng = self.passw_mng.borrow().clone();
            if let Some(m) = mng {
                m.lock_if_open();
            }
        }
        if self.core.borrow().config.minimize_to_sys_tray && !self.tray.hidden.get() {
            self.tray.hidden.set(true);
            self.update_tray_icon();
            // only hide if the tray icon is shown by a tray host
            if self.tray.icon.borrow().as_ref().is_some_and(|t| t.is_online()) {
                self.window.set_visible(false);
            } else {
                self.tray.hidden.set(false);
            }
        }
    }

    /// Show the main window again (`RestoreAction()` / `AppRestore()`).
    pub fn restore_from_tray(&self) {
        self.tray.hidden.set(false);
        self.window.set_visible(true);
        self.window.unminimize();
        self.window.present();
        self.update_tray_icon();
    }

    /// `TrayIconDblClick()`.
    fn tray_activate(&self) {
        if self.tray.hidden.get() || !self.window.is_visible() {
            self.restore_from_tray();
        } else {
            self.window.present();
        }
    }

    fn tray_profile(&self, i: usize) {
        let Some(name) = self.core.borrow().profiles.get(i).map(|p| p.profile_name.clone()) else { return };
        self.load_profile(i);
        let m = trl_format("Profile \"%1\" activated.", &[&name]);
        if self.tray.hidden.get() {
            show_notification(&self.window, &m, false);
        } else {
            info_box(&self.win(), &m);
        }
    }

    /// Hide the main window at start-up (`--silent`).
    pub fn start_hidden(self: &Rc<Self>) {
        self.tray.hidden.set(true);
        self.update_tray_icon();
        // no tray host: the window must be reachable
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            if let Some(w) = w.upgrade() {
                if w.tray.hidden.get() && !w.tray.icon.borrow().as_ref().is_some_and(|t| t.is_online()) {
                    w.restore_from_tray();
                }
            }
        });
    }

    /// Show/hide the tray icon according to the configuration
    /// (`TrayIcon->Visible = ...`) and update icon, title and profiles.
    pub(super) fn update_tray_icon(&self) {
        let (visible, icon_name) = {
            let core = self.core.borrow();
            (core.config.show_sys_tray_icon_const || self.tray.hidden.get(), core.config.app_icon_name.clone())
        };
        let title = self.window.title().map(|t| t.to_string()).unwrap_or_else(|| PROGRAM_NAME.to_string());
        let profiles: Vec<String> = self.core.borrow().profiles.iter().map(|p| p.profile_name.clone()).collect();
        let mut icon = self.tray.icon.borrow_mut();
        if !visible {
            icon.take();
            return;
        }
        match icon.as_ref() {
            None => {
                if self.tray.last_spawn_failure.get().is_some_and(|t| t.elapsed().as_secs() < 60) {
                    return;
                }
                *icon = TrayIcon::spawn(
                    &title,
                    icon_from_pngs(&app_icon_pngs(&icon_name)),
                    self.tray_entries(),
                    profiles,
                    self.tray.tx.clone(),
                );
                if icon.is_none() {
                    self.tray.last_spawn_failure.set(Some(std::time::Instant::now()));
                }
                *self.tray.title.borrow_mut() = title;
                *self.tray.icon_name.borrow_mut() = icon_name;
            }
            Some(t) => {
                if *self.tray.title.borrow() != title {
                    t.set_title(&title);
                    *self.tray.title.borrow_mut() = title;
                }
                if *self.tray.icon_name.borrow() != icon_name {
                    t.set_icon(icon_from_pngs(&app_icon_pngs(&icon_name)));
                    *self.tray.icon_name.borrow_mut() = icon_name;
                }
                t.set_profiles(profiles);
            }
        }
    }

    /// Called by the main timer: keep the tray title in sync; disable the
    /// tray menu while a modal dialog is open or a generation is running
    /// (`TrayMenuPopup()`).
    pub(super) fn tray_timer(&self) {
        if let Some(t) = self.tray.icon.borrow().as_ref() {
            let busy = self.generating.get()
                || gtk::Window::list_toplevels()
                    .into_iter()
                    .filter_map(|x| x.downcast::<gtk::Window>().ok())
                    .any(|x| x.is_visible() && x.is_modal());
            if busy != self.tray.menu_disabled.get() {
                self.tray.menu_disabled.set(busy);
                t.set_enabled(!busy);
            }
            let title = self.window.title().map(|t| t.to_string()).unwrap_or_default();
            if *self.tray.title.borrow() != title {
                t.set_title(&title);
                *self.tray.title.borrow_mut() = title;
            }
        }
    }
}

/// `ShowTrayInfo()`: desktop notification while the window is hidden.
pub fn show_notification(window: &gtk::ApplicationWindow, text: &str, error: bool) {
    let Some(app) = window.application() else { return };
    let n = gio::Notification::new(PROGRAM_NAME);
    n.set_body(Some(text));
    if error {
        n.set_priority(gio::NotificationPriority::High);
    }
    app.send_notification(Some("pwtool-info"), &n);
}
