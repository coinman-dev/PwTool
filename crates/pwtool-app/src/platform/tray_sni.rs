//! System tray icon (`TTrayIcon` with `TrayMenu` of the main form) as a
//! StatusNotifierItem (shown by KDE Plasma and other desktops with SNI
//! support). The tray runs in its own thread; commands are sent to the GTK
//! main loop through a channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use ksni::blocking::TrayMethods;
use ksni::menu::{StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip};

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

pub struct PwTray {
    title: String,
    icon: Vec<Icon>,
    entries: Vec<TrayEntry>,
    profiles: Vec<String>,
    enabled: bool,
    tx: async_channel::Sender<TrayCmd>,
    online: Arc<AtomicBool>,
}

impl ksni::Tray for PwTray {
    fn watcher_online(&self) {
        self.online.store(true, Ordering::Relaxed);
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.online.store(false, Ordering::Relaxed);
        true
    }

    fn id(&self) -> String {
        "pwtool".into()
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: self.title.clone(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.try_send(TrayCmd::Activate);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut v: Vec<MenuItem<Self>> = Vec::new();
        for e in &self.entries {
            match e {
                TrayEntry::Separator => v.push(MenuItem::Separator),
                TrayEntry::Item { name, label, bold } => {
                    let name = name.clone();
                    v.push(
                        StandardItem {
                            label: label.clone(),
                            enabled: self.enabled,
                            disposition: if *bold {
                                ksni::menu::Disposition::Informative
                            } else {
                                ksni::menu::Disposition::Normal
                            },
                            activate: Box::new(move |t: &mut Self| {
                                let _ = t.tx.try_send(TrayCmd::Menu(name.clone()));
                            }),
                            ..Default::default()
                        }
                        .into(),
                    );
                }
                TrayEntry::Profiles { label, rest } => {
                    let mut sub: Vec<MenuItem<Self>> = Vec::new();
                    let shortcuts: Vec<char> = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars().collect();
                    for (i, p) in self.profiles.iter().enumerate() {
                        let mut l = String::new();
                        if let Some(c) = shortcuts.get(i) {
                            l.push('_');
                            l.push(*c);
                            l.push(' ');
                        }
                        l.push_str(&p.replace('_', "__"));
                        sub.push(
                            StandardItem {
                                label: l,
                                activate: Box::new(move |t: &mut Self| {
                                    let _ = t.tx.try_send(TrayCmd::Profile(i));
                                }),
                                ..Default::default()
                            }
                            .into(),
                        );
                    }
                    if !self.profiles.is_empty() {
                        sub.push(MenuItem::Separator);
                    }
                    for (name, l) in rest {
                        let name = name.clone();
                        sub.push(
                            StandardItem {
                                label: l.clone(),
                                activate: Box::new(move |t: &mut Self| {
                                    let _ = t.tx.try_send(TrayCmd::Menu(name.clone()));
                                }),
                                ..Default::default()
                            }
                            .into(),
                        );
                    }
                    v.push(SubMenu { label: label.clone(), enabled: self.enabled, submenu: sub, ..Default::default() }.into());
                }
            }
        }
        v
    }
}

/// A running tray icon.
pub struct TrayIcon {
    handle: ksni::blocking::Handle<PwTray>,
    online: Arc<AtomicBool>,
}

impl TrayIcon {
    /// Show the tray icon. `None` if no StatusNotifierWatcher is available
    /// (errors are not fatal: the program works without tray).
    pub fn spawn(
        title: &str,
        icon: Vec<Icon>,
        entries: Vec<TrayEntry>,
        profiles: Vec<String>,
        tx: async_channel::Sender<TrayCmd>,
    ) -> Option<TrayIcon> {
        let online = Arc::new(AtomicBool::new(true));
        let tray = PwTray { title: title.to_string(), icon, entries, profiles, enabled: true, tx, online: online.clone() };
        match tray.spawn() {
            Ok(handle) => Some(TrayIcon { handle, online }),
            Err(e) => {
                eprintln!("pwtool: tray icon not available: {e}");
                None
            }
        }
    }

    /// Whether a tray host (StatusNotifierWatcher) shows the icon.
    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    pub fn set_title(&self, title: &str) {
        let t = title.to_string();
        self.handle.update(move |tr| tr.title = t);
    }

    pub fn set_profiles(&self, profiles: Vec<String>) {
        self.handle.update(move |tr| tr.profiles = profiles);
    }

    pub fn set_icon(&self, icon: Vec<Icon>) {
        self.handle.update(move |tr| tr.icon = icon);
    }

    /// `TrayMenuPopup()`: items are disabled while a modal dialog is open.
    pub fn set_enabled(&self, enabled: bool) {
        self.handle.update(move |tr| tr.enabled = enabled);
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        let _ = self.handle.shutdown();
    }
}

/// Icon for the tray from PNG images (ARGB32, network byte order), one
/// entry per size.
pub fn icon_from_pngs(images: &[&'static [u8]]) -> Vec<Icon> {
    images
        .iter()
        .filter_map(|b| crate::icons::decode_png(b))
        .map(|img| Icon {
            width: img.width as i32,
            height: img.height as i32,
            // ARGB32 in network byte order
            data: img.pixels.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect(),
        })
        .collect()
}
