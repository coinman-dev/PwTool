//! The program's windows on top of `vcl`.

pub mod automate;
pub mod clipboard;
pub mod dialogs;
pub mod main_window;
pub mod quality_bar;

use std::rc::Rc;

use egui::{ViewportCommand, ViewportId};
use pwtool_app::app::{AppCore, CmdLineOptions};

use crate::vcl::fonts::{self, UserFont};
use crate::windows;
use main_window::{MainContent, MainWindow};

struct App {
    main: Option<Rc<MainWindow>>,
    cmd: CmdLineOptions,
    closed: bool,
}

impl eframe::App for App {
    /// Also called while the main window is minimized or hidden (then
    /// `ui` is not): tasks (tray commands, hot keys, timers) keep running.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::runtime::poll();
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
        let main = self
            .main
            .get_or_insert_with(|| {
                let w = MainWindow::new(AppCore::new(self.cmd.clone()));
                let size = crate::vcl::window_size(&w.form, &ctx);
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                if let Ok(script) = std::env::var("PWTOOL_AUTOMATE") {
                    automate::start(w.clone(), script);
                }
                if self.cmd.silent {
                    w.start_hidden();
                }
                w
            })
            .clone();
        automate::save_screenshots(&ctx);
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
    #[cfg(windows)]
    let Some(instance) = pwtool_app::platform::single_instance::Instance::acquire() else { return 0 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(pwtool_app::program::PROGRAM_NAME)
            .with_app_id(pwtool_app::program::APP_ID)
            .with_inner_size([560.0, 700.0])
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };
    let r = eframe::run_native(
        pwtool_app::program::APP_ID,
        options,
        Box::new(move |cc| {
            fonts::install(&cc.egui_ctx, &UserFont::default());
            cc.egui_ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::System);
            crate::runtime::install(&cc.egui_ctx);
            windows::set_context(&cc.egui_ctx);
            #[cfg(windows)]
            {
                let ctx = cc.egui_ctx.clone();
                instance.watch(move || {
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                });
            }
            Ok(Box::new(App { main: None, cmd, closed: false }))
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
