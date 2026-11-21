//! The program's windows on top of `vcl`.

pub mod automate;
pub mod clipboard;
pub mod dialogs;
pub mod font_dialog;
pub mod main_window;
pub mod passw_enter;
pub mod quality_bar;
pub mod update_check;

use std::rc::Rc;

use egui::{ViewportCommand, ViewportId};
use pwtool_app::app::{AppCore, CmdLineOptions};

use crate::vcl::fonts::{self, FontSpec};
use crate::windows;
use main_window::config_dlg::{app_icon, apply_ui_style};
use main_window::{MainContent, MainWindow};

struct App {
    main: Option<Rc<MainWindow>>,
    core: Option<AppCore>,
    silent: bool,
    /// Size the main window was created with.
    initial_size: egui::Vec2,
    closed: bool,
    /// Kept while the program runs (one instance per user).
    _instance: pwtool_app::platform::single_instance::Instance,
    /// Set when another start of the program was attempted.
    activate: Rc<std::cell::Cell<bool>>,
}

impl eframe::App for App {
    /// Also called while the main window is minimized or hidden (then
    /// `ui` is not): tasks (tray commands, hot keys, timers) keep running.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::runtime::poll();
        if let (true, Some(m)) = (self.activate.take(), &self.main) {
            m.restore_from_tray();
        }
        if let Some(m) = &self.main {
            m.watch_window(ctx);
        }
        // keep the timer tasks going while no frames are drawn
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        windows::set_root_window(frame);
        crate::runtime::poll();
        let main = match &self.main {
            Some(m) => m.clone(),
            None => {
                let Some(core) = self.core.take() else { return };
                let w = MainWindow::new(core);
                let size = crate::vcl::window_size(&w.form, &ctx);
                if (size - self.initial_size).abs().max_elem() > 1.0 {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                }
                if let Ok(script) = std::env::var("PWTOOL_AUTOMATE") {
                    automate::start(w.clone(), script);
                }
                if self.silent {
                    w.start_hidden();
                }
                self.main = Some(w.clone());
                w
            }
        };

        windows::show(ui, ViewportId::ROOT, &MainContent(main.clone()));
        if !main.is_hidden() {
            main.show_startup_errors();
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.closed {
            if main.close_query() {
                main.on_close();
                self.closed = true;
            } else {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
        }
    }
}

/// Run the GUI. Returns the exit code.
pub fn run(cmd: CmdLineOptions) -> i32 {
    let Some(instance) = pwtool_app::platform::single_instance::Instance::acquire() else { return 0 };
    let silent = cmd.silent;
    // the configuration first: font, colour scheme and icon of the windows
    let core = AppCore::new(cmd);
    let user_font = FontSpec::parse(&core.config.gui_font_string).map(|f| f.user_font()).unwrap_or_default();
    let style = core.config.ui_style_name.clone();
    // the main window gets the size of its form at once: resizing it
    // after creation upsets some Wayland compositors (WSLg)
    let initial_size = main_window_size(&user_font);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(pwtool_app::program::PROGRAM_NAME)
        .with_app_id(pwtool_app::program::APP_ID)
        .with_inner_size(initial_size)
        .with_resizable(false)
        .with_maximize_button(false);
    if let Some(icon) = app_icon(&core.config.app_icon_name) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let r = eframe::run_native(
        pwtool_app::program::APP_ID,
        options,
        Box::new(move |cc| {
            fonts::install(&cc.egui_ctx, &user_font);
            apply_ui_style(&cc.egui_ctx, &style);
            crate::runtime::install(&cc.egui_ctx);
            windows::set_context(&cc.egui_ctx);

            // another start: show the main window (also from the tray)
            let activate = Rc::new(std::cell::Cell::new(false));
            let (a, ctx) = (activate.clone(), cc.egui_ctx.clone());
            instance.watch(move || {
                a.set(true);
                ctx.request_repaint();
            });
            Ok(Box::new(App { main: None, core: Some(core), silent, initial_size, closed: false, activate, _instance: instance }))
        }),
    );
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pwtool: {e}");
            1
        }
    }
}

/// Size of the main window's form, measured with the fonts in a context of
/// its own before the window exists.
fn main_window_size(user_font: &fonts::UserFont) -> egui::Vec2 {
    let ctx = egui::Context::default();
    fonts::install(&ctx, user_font);
    // the first pass loads the fonts
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let form = crate::vcl::load_form(&ctx, "Main");
    crate::vcl::window_size(&form, &ctx)
}
