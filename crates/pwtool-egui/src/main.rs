//! Password Tool with an egui interface (work in progress, see
//! `.ai/PLAN-egui.md`). For now: drawing the forms of the original.
//!
//!   pwtool-egui [--show FORM] [--lang LANGUAGE] [--dark]
//!   pwtool-egui --forms DIR [--lang LANGUAGE] [--dark] [--ppp 1.25]
//!
//! `--forms` writes a picture of every form to DIR without opening a window.

// GUI program on Windows (no console window)
#![cfg_attr(windows, windows_subsystem = "windows")]
// The vcl API is complete; the windows using all of it are ported in stage 3.
#![allow(dead_code)]

mod runtime;
mod vcl;
mod windows;

use std::path::{Path, PathBuf};

use vcl::fonts::{self, UserFont};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn assets() -> PathBuf {
    if let Some(d) = std::env::var_os("PWTOOL_DATA_DIR") {
        return d.into();
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// Load the translation `name` ("Russian", "German"…) from the assets.
fn load_language(name: &str) {
    use pwtool_core::language_file::LanguageFile;
    let dir = assets().join("languages");
    let file = ["po", "lng"].iter().map(|e| dir.join(format!("{name}.{e}"))).find(|p| p.is_file());
    let Some(file) = file else {
        eprintln!("pwtool-egui: language {name} not found in {}", dir.display());
        return;
    };
    match LanguageFile::load(&file, false) {
        Ok(mut lf) => {
            if let Ok(x) = LanguageFile::load(&dir.join("port").join(format!("{}.po", lf.code)), false) {
                lf.merge_missing(x);
            }
            pwtool_core::language::set_language(Some(lf));
        }
        Err(e) => eprintln!("pwtool-egui: {}: {e:?}", file.display()),
    }
}

/// Pictures of all forms (`--forms DIR`).
fn snapshot_forms(args: &[String]) -> std::io::Result<()> {
    let dir = PathBuf::from(arg(args, "--forms").unwrap_or_else(|| "forms".into()));
    std::fs::create_dir_all(&dir)?;
    let ppp: f32 = arg(args, "--ppp").and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let ctx = egui::Context::default();
    fonts::install(&ctx, &UserFont::default());
    ctx.set_theme(if args.iter().any(|a| a == "--dark") { egui::Theme::Dark } else { egui::Theme::Light });
    let mut snap = vcl::snapshot::Snapshot::new(ctx.clone());
    // the first frame loads the fonts
    snap.render(egui::vec2(100.0, 100.0), ppp, 1, |_| {});
    let only = arg(args, "--form");
    for name in vcl::FORMS.iter().filter(|n| only.as_deref().is_none_or(|o| o == **n)) {
        let form = vcl::load_form(&ctx, name);
        let size = vcl::window_size(&form, &ctx);
        let img = snap.render(size, ppp, 3, |ui| {
            let fill = ui.visuals().panel_fill;
            ui.painter().rect_filled(ui.max_rect(), 0.0, fill);
            form.show(ui);
        });
        let path = dir.join(format!("{name}.png"));
        vcl::snapshot::save_png(&img, &path)?;
        println!("{} {}x{}", path.display(), img.size[0], img.size[1]);
    }
    Ok(())
}

/// Window showing one form (`--show FORM`), for checking the drawing and
/// the window infrastructure: F2 next form, F3 message box, F4 next form
/// as a modal dialog, F5 file dialog. `PWTOOL_DEMO=1` opens a dialog and a
/// message box, reports and quits.
struct Viewer {
    name: String,
    form: Option<std::rc::Rc<vcl::Form>>,
    started: std::time::Instant,
    demo: bool,
    demo_step: u32,
}

impl Viewer {
    fn open_dialog(&self, ctx: &egui::Context, name: &str) {
        let f = std::rc::Rc::new(vcl::load_form(ctx, name));
        let size = vcl::window_size(&f, ctx);
        let title = vcl::render::strip_amp(&f.caption());
        let id = format!("dlg:{name}");
        let vid = egui::ViewportId::from_hash_of(&id);
        let w = windows::Window::new(&id, &title, f.clone(), egui::ViewportId::ROOT, size).modal(true);
        windows::open(std::rc::Rc::new(w));
        // OK/Cancel/Close buttons close it
        let f2 = f.clone();
        runtime::spawn(async move {
            loop {
                pwtool_app::rt::sleep(std::time::Duration::from_millis(50)).await;
                if !windows::is_open(vid) {
                    return;
                }
                for e in f2.take_events() {
                    println!("dialog {e:?}");
                    if let vcl::model::Event::Click(n) = &e {
                        if n.contains("OK") || n.contains("Cancel") || n.contains("Close") {
                            windows::close(vid);
                            return;
                        }
                    }
                }
            }
        });
    }
}

impl eframe::App for Viewer {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        windows::set_root_window(frame);
        runtime::poll();
        if self.form.is_none() {
            let f = vcl::load_form(&ctx, &self.name);
            let size = vcl::window_size(&f, &ctx);
            let title = vcl::render::strip_amp(&f.caption());
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.form = Some(std::rc::Rc::new(f));
        }
        let form = self.form.clone().unwrap();
        windows::show(ui, egui::ViewportId::ROOT, &*form);
        for e in form.take_events() {
            println!("{e:?}");
        }
        let (f2, f3, f4, f5) = ctx.input(|i| {
            (i.key_pressed(egui::Key::F2), i.key_pressed(egui::Key::F3), i.key_pressed(egui::Key::F4), i.key_pressed(egui::Key::F5))
        });
        let next = vcl::FORMS[(vcl::FORMS.iter().position(|n| *n == self.name).unwrap_or(0) + 1) % vcl::FORMS.len()];
        if f2 {
            self.name = next.to_string();
            self.form = None;
        }
        if f3 {
            let c = ctx.clone();
            runtime::spawn(async move {
                let r = vcl::dialogs::msg_box(
                    &c,
                    egui::ViewportId::ROOT,
                    "Do you want to save the changes?
Сохранить изменения? 保存更改？",
                    vcl::dialogs::MsgIcon::Question,
                    vcl::dialogs::MsgButtons::YesNoCancel,
                    false,
                )
                .await;
                println!("msg_box: {r:?}");
            });
        }
        if f4 {
            self.open_dialog(&ctx, next);
        }
        if f5 {
            runtime::spawn(async move {
                let p = vcl::dialogs::open_file("Open", None, &[("Password databases", &["pwdb"]), ("All files", &["*"])]).await;
                println!("open_file: {p:?}");
            });
        }
        if self.demo {
            self.run_demo(&ctx);
        }
    }
}

impl Viewer {
    /// Automatic check of the window infrastructure (`PWTOOL_DEMO`).
    fn run_demo(&mut self, ctx: &egui::Context) {
        let t = self.started.elapsed().as_secs_f32();
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        match self.demo_step {
            0 if t > 0.5 => {
                self.open_dialog(ctx, "PasswOptions");
                self.demo_step = 1;
            }
            1 if t > 1.5 => {
                let id = egui::ViewportId::from_hash_of("dlg:PasswOptions");
                println!("demo: dialog open={} modal child of root={:?}", windows::is_open(id), windows::modal_child(egui::ViewportId::ROOT) == Some(id));
                #[cfg(windows)]
                println!("demo: owner of dialog is main window: {}", owner_check("Advanced Password Options"));
                windows::close(id);
                let c = ctx.clone();
                runtime::spawn(async move {
                    let r = vcl::dialogs::msg_box(&c, egui::ViewportId::ROOT, "Message box test", vcl::dialogs::MsgIcon::Info, vcl::dialogs::MsgButtons::OkCancel, false).await;
                    println!("demo: msg_box result {r:?}");
                });
                self.demo_step = 2;
            }
            2 if t > 2.5 => {
                // press Escape in the message box
                println!("demo: open windows with modal: {}", windows::any_modal());
                self.demo_step = 3;
            }
            3 if t > 3.0 => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.demo_step = 4;
            }
            _ => {}
        }
    }
}

#[cfg(windows)]
fn owner_check(dialog: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GW_OWNER, GetWindow};
    let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    // SAFETY: plain Win32 lookups.
    unsafe {
        let d = FindWindowW(std::ptr::null(), w(dialog).as_ptr());
        let m = windows::root_hwnd() as windows_sys::Win32::Foundation::HWND;
        let o = if d.is_null() { std::ptr::null_mut() } else { GetWindow(d, GW_OWNER) };
        println!("demo: dialog={d:?} main={m:?} owner={o:?}");
        !d.is_null() && !m.is_null() && o == m
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(lang) = arg(&args, "--lang") {
        load_language(&lang);
    }
    if args.iter().any(|a| a == "--forms") {
        if let Err(e) = snapshot_forms(&args) {
            eprintln!("pwtool-egui: {e}");
            std::process::exit(1);
        }
        return;
    }
    let name = arg(&args, "--show").unwrap_or_else(|| "Main".into());
    let dark = args.iter().any(|a| a == "--dark");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Password Tool").with_inner_size([600.0, 700.0]),
        ..Default::default()
    };
    let r = eframe::run_native(
        "pwtool-egui",
        options,
        Box::new(move |cc| {
            fonts::install(&cc.egui_ctx, &UserFont::default());
            cc.egui_ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
            runtime::install(&cc.egui_ctx);
            windows::set_context(&cc.egui_ctx);
            let demo = std::env::var_os("PWTOOL_DEMO").is_some();
            Ok(Box::new(Viewer { name, form: None, started: std::time::Instant::now(), demo, demo_step: 0 }))
        }),
    );
    if let Err(e) = r {
        eprintln!("pwtool-egui: {e}");
    }
}
