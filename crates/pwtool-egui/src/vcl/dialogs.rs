//! Standard dialogs: message boxes (`MsgBox()` of the original) and the
//! file dialogs of the system (Windows file dialog, XDG portal on Linux).

use std::cell::Cell;
use std::rc::Rc;

use egui::{Align2, Color32, FontId, Key, RichText, Ui, Vec2, ViewportId, vec2};
use pwtool_core::language::trl;

use crate::windows::{self, Content, Window};

/// Icon/kind of a message box (`MB_ICON…`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgIcon {
    Info,
    Warning,
    Error,
    Question,
}

/// Buttons of a message box (`MB_OK`, `MB_YESNO`…).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgButtons {
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
}

/// Result of a message box (`IDOK`, `IDYES`…).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgResult {
    Ok,
    Cancel,
    Yes,
    No,
}

struct MsgBox {
    id: ViewportId,
    text: String,
    icon: MsgIcon,
    buttons: Vec<(String, MsgResult)>,
    default: usize,
    escape: MsgResult,
    result: Rc<Cell<Option<MsgResult>>>,
    tx: async_channel::Sender<()>,
}

impl MsgBox {
    fn finish(&self, r: MsgResult) {
        self.result.set(Some(r));
        windows::close(self.id);
        let _ = self.tx.try_send(());
    }
}

const ICON_SIZE: f32 = 36.0;
const MARGIN: f32 = 16.0;
const TEXT_WIDTH: f32 = 460.0;
const BUTTON: Vec2 = vec2(88.0, 26.0);

impl Content for MsgBox {
    fn ui(&self, ui: &mut Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill);
        let (glyph, color) = match self.icon {
            MsgIcon::Info => ("i", Color32::from_rgb(0x1e, 0x88, 0xe5)),
            MsgIcon::Warning => ("!", Color32::from_rgb(0xf5, 0xa6, 0x23)),
            MsgIcon::Error => ("×", Color32::from_rgb(0xe5, 0x39, 0x35)),
            MsgIcon::Question => ("?", Color32::from_rgb(0x1e, 0x88, 0xe5)),
        };
        let c = rect.min + vec2(MARGIN + ICON_SIZE / 2.0, MARGIN + ICON_SIZE / 2.0);
        ui.painter().circle_filled(c, ICON_SIZE / 2.0, color);
        ui.painter().text(c, Align2::CENTER_CENTER, glyph, FontId::proportional(ICON_SIZE * 0.7), Color32::WHITE);
        let text_rect = egui::Rect::from_min_max(
            rect.min + vec2(2.0 * MARGIN + ICON_SIZE, MARGIN),
            rect.max - vec2(MARGIN, 2.0 * MARGIN + BUTTON.y),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
            ui.add(egui::Label::new(&self.text).wrap().selectable(true));
        });
        // buttons, right-aligned
        let n = self.buttons.len() as f32;
        let mut x = rect.max.x - MARGIN - n * BUTTON.x - (n - 1.0) * 8.0;
        let y = rect.max.y - MARGIN - BUTTON.y;
        for (i, (label, r)) in self.buttons.iter().enumerate() {
            let br = egui::Rect::from_min_size(egui::pos2(x, y), BUTTON);
            let b = ui.put(br, egui::Button::new(RichText::new(crate::vcl::render::strip_amp(label))));
            if i == self.default && !ui.memory(|m| m.focused().is_some()) {
                b.request_focus();
            }
            if b.clicked() {
                self.finish(*r);
            }
            x += BUTTON.x + 8.0;
        }
        let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if esc {
            self.finish(self.escape);
        } else if enter && ui.memory(|m| m.focused().is_none()) {
            self.finish(self.buttons[self.default].1);
        }
    }
}

thread_local! {
    static COUNTER: Cell<u64> = const { Cell::new(0) };
}

/// `MsgBox()`: modal message box over window `parent`; `default_second`
/// makes the second button the default (`MB_DEFBUTTON2`).
pub async fn msg_box(ctx: &egui::Context, parent: ViewportId, text: &str, icon: MsgIcon, buttons: MsgButtons, default_second: bool) -> MsgResult {
    use MsgResult::*;
    let list: Vec<(String, MsgResult)> = match buttons {
        MsgButtons::Ok => vec![(trl("OK"), Ok)],
        MsgButtons::OkCancel => vec![(trl("OK"), Ok), (trl("Cancel"), Cancel)],
        MsgButtons::YesNo => vec![(format!("&{}", trl("Yes")), Yes), (format!("&{}", trl("No")), No)],
        MsgButtons::YesNoCancel => vec![(format!("&{}", trl("Yes")), Yes), (format!("&{}", trl("No")), No), (trl("Cancel"), Cancel)],
    };
    let escape = match buttons {
        MsgButtons::Ok => Ok,
        MsgButtons::YesNo => No,
        _ => Cancel,
    };
    let n = COUNTER.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let id = format!("msgbox{n}");
    // size from the text
    // (the style must be read before locking the fonts)
    let font = egui::TextStyle::Body.resolve(&ctx.global_style());
    let galley = ctx.fonts_mut(|f| f.layout(text.to_string(), font, Color32::WHITE, TEXT_WIDTH));
    let text_size = galley.size();
    let buttons_w = list.len() as f32 * (BUTTON.x + 8.0) + 2.0 * MARGIN;
    let w = (3.0 * MARGIN + ICON_SIZE + text_size.x.max(120.0)).max(buttons_w);
    let h = MARGIN + text_size.y.max(ICON_SIZE) + MARGIN + BUTTON.y + MARGIN;
    let (tx, rx) = async_channel::bounded::<()>(1);
    let result = Rc::new(Cell::new(None));
    let content = Rc::new(MsgBox {
        id: ViewportId::from_hash_of(&id),
        text: text.to_string(),
        icon,
        buttons: list,
        default: if default_second && buttons != MsgButtons::Ok { 1 } else { 0 },
        escape,
        result: result.clone(),
        tx: tx.clone(),
    });
    let win = Window::new(&id, pwtool_app::program::PROGRAM_NAME, content, parent, vec2(w, h)).modal(true);
    let r2 = result.clone();
    *win.on_close.borrow_mut() = Some(Box::new(move || {
        if r2.get().is_none() {
            r2.set(Some(escape));
        }
        let _ = tx.try_send(());
        true
    }));
    windows::open(Rc::new(win));
    let _ = rx.recv().await;
    result.get().unwrap_or(escape)
}

/// File type filter of a file dialog: (name, extensions without dot).
pub type Filter<'a> = (&'a str, &'a [&'a str]);

fn file_dialog(title: &str, dir: Option<&std::path::Path>, name: Option<&str>, filters: &[Filter]) -> rfd::AsyncFileDialog {
    let mut d = rfd::AsyncFileDialog::new().set_title(title);
    for (n, ext) in filters {
        d = d.add_filter(*n, ext);
    }
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    if let Some(n) = name {
        d = d.set_file_name(n);
    }
    d
}

/// Choose a file to open.
pub async fn open_file(title: &str, dir: Option<&std::path::Path>, filters: &[Filter<'_>]) -> Option<std::path::PathBuf> {
    file_dialog(title, dir, None, filters).pick_file().await.map(|f| f.path().to_path_buf())
}

/// Choose a file name to save to.
pub async fn save_file(title: &str, dir: Option<&std::path::Path>, name: Option<&str>, filters: &[Filter<'_>]) -> Option<std::path::PathBuf> {
    file_dialog(title, dir, name, filters).save_file().await.map(|f| f.path().to_path_buf())
}
