//! Password Tool: password generator and password manager, a port of
//! Password Tech 3.6.1. The windows are the forms of the original
//! (`assets/forms`) drawn with egui (`vcl`); the program logic lives in
//! `ui`, the toolkit-independent part in `pwtool-app` and `pwtool-core`.
//!
//! Developer check (not in `--help`): pictures of all forms without a window
//!
//!   pwtool --forms DIR [--lang LANGUAGE] [--dark] [--ppp 1.25] [--form NAME] [--fit]
//!
//! `--fit` also lists captions that do not fit or lack glyphs.

// GUI program on Windows (no console window)
#![cfg_attr(windows, windows_subsystem = "windows")]

mod runtime;
mod ui;
mod vcl;
mod windows;

use std::path::PathBuf;

use vcl::fonts::{self, UserFont};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// Load the translation `name` ("Russian", "German"…) for `--forms`.
fn load_language(name: &str) {
    use pwtool_core::language_file::LanguageFile;
    let data = std::env::var_os("PWTOOL_DATA_DIR").map(PathBuf::from).or_else(pwtool_app::resources::extract);
    let Some(dir) = data.map(|d| d.join("languages")) else { return };
    let file = ["po", "lng"].iter().map(|e| dir.join(format!("{name}.{e}"))).find(|p| p.is_file());
    let Some(file) = file else {
        eprintln!("pwtool: language {name} not found in {}", dir.display());
        return;
    };
    match LanguageFile::load(&file, false) {
        Ok(mut lf) => {
            if let Ok(x) = LanguageFile::load(&dir.join("port").join(format!("{}.po", lf.code)), false) {
                lf.merge_missing(x);
            }
            pwtool_core::language::set_language(Some(lf));
        }
        Err(e) => eprintln!("pwtool: {}: {e:?}", file.display()),
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
    let fit = args.iter().any(|a| a == "--fit");
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
        if fit {
            for line in vcl::render::fit_report(&form, &ctx) {
                println!("fit: {line}");
            }
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--forms") {
        if let Some(lang) = arg(&args, "--lang") {
            load_language(&lang);
        }
        if let Err(e) = snapshot_forms(&args) {
            eprintln!("pwtool: {e}");
            std::process::exit(1);
        }
        return;
    }
    // console mode or GUI
    pwtool_core::secure_mem::harden_process();
    let cmd = pwtool_app::app::parse_cmdline(&args);
    if cmd.gen_num_passw > 0 || cmd.show_help {
        #[cfg(windows)]
        pwtool_app::platform::sys::attach_parent_console();
        std::process::exit(pwtool_app::cli::run(cmd));
    }
    let rc = ui::run(cmd);
    if pwtool_app::app::restart_requested() {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
    }
    std::process::exit(rc);
}
