//! Console mode (`-pg`, `-help`): `TMainForm::StartupAction()` and
//! `GeneratePassw(gpdConsole)`.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64};

use pwtech_core::generate::{GenDest, SinkAction, generate};
use pwtech_core::language::{trl, trl_format};
use pwtech_core::random::RandomPool;
use pwtech_core::scripting::LuaScript;

use crate::app::{AppCore, CmdLineOptions};
use crate::program::*;

/// Run the console mode. Returns the process exit code.
pub fn run(cmd: CmdLineOptions) -> i32 {
    let mut app = AppCore::new(cmd);
    let out = std::io::stdout();
    let mut out = out.lock();
    let _ = writeln!(out, "This is {PROGRAM_NAME} version {}.", version_string());

    if app.cmd.show_help {
        let _ = writeln!(out, "{}", AppCore::cmd_line_info());
        return 0;
    }

    for e in app.startup_errors.drain(..) {
        eprintln!("{e}");
    }

    let mut profile = app.cmd.profile_name.clone();
    if profile.is_empty() && app.config.load_profile_startup {
        profile = app.config.load_profile_name.clone();
    }
    if !profile.is_empty() && !app.load_profile_by_name(&profile) {
        let _ = writeln!(out, "{} - {}", trl("Error"), trl_format("Profile \"%1\" not found.", &[&profile]));
        app.cmd.profile_name.clear();
    }

    let num = app.cmd.gen_num_passw;
    if !app.cmd.profile_name.is_empty() {
        let _ = writeln!(out, "{}", trl_format("Selected profile \"%1\".", &[&app.cmd.profile_name]));
    }
    let _ = writeln!(out, "{}\n", trl_format("Generating %1 password(s) ...", &[&num.to_string()]));

    let req = match app.gen_request(GenDest::Console, num) {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(out, "{e}");
            return 1;
        }
    };

    // script
    let mut script: Option<LuaScript> = None;
    if let Some(path) = app.script_file() {
        let mut s = LuaScript::new();
        let mut pool = RandomPool::global();
        if let Err(e) = s.load_file(&path, Some(&app.passw_gen), &mut *pool) {
            drop(pool);
            let _ = writeln!(out, "{}", trl_format("Error while loading script:\n%1.", &[&e.to_string()]));
            return 1;
        }
        script = Some(s);
    }

    let cancel = AtomicBool::new(false);
    let progress = AtomicU64::new(0);
    let mut pool = RandomPool::global();
    let pg = &app.passw_gen;
    let mut gen_flags = 0;
    if req.chars_len != 0 {
        gen_flags |= 1;
    }
    if req.num_words != 0 {
        gen_flags |= 2;
    }
    if !req.format.is_empty() {
        gen_flags |= 4;
    }
    let format_text = req.format.clone();
    let standalone = script
        .as_ref()
        .map(|s| s.flags() & pwtech_core::scripting::FLAG_STANDALONE_PASSW_GEN != 0)
        .unwrap_or(false);
    let has_script = script.is_some();
    let mut hook_state = script.as_mut().map(|s| {
        let standalone = s.flags() & pwtech_core::scripting::FLAG_STANDALONE_PASSW_GEN != 0;
        (s, standalone)
    });
    let mut hook = |rng: &mut dyn pwtech_core::random::RandomGenerator,
                    n: u64,
                    p: Option<&str>,
                    sec: f64|
     -> Result<(zeroize::Zeroizing<String>, f64), String> {
        let (s, _) = hook_state.as_mut().unwrap();
        if !s.is_initialized() {
            s.init_generate(
                pg,
                rng,
                num,
                6,
                gen_flags,
                req.option_flags,
                req.chars_len,
                req.num_words,
                &format_text,
            )
            .map_err(|e| e.to_string())?;
        }
        s.call_generate(pg, rng, n, p, sec).map_err(|e| e.to_string())
    };
    let script_arg: Option<(&mut pwtech_core::generate::ScriptHook<'_>, bool)> =
        if has_script { Some((&mut hook, standalone)) } else { None };

    let res = generate(pg, &mut *pool, &req, app.common_passw.as_ref(), script_arg, &cancel, &progress, &mut |p, _| {
        let _ = writeln!(out, "{}", p.passw);
        Ok(SinkAction::Continue)
    });
    pool.flush();
    drop(pool);
    if let Err(e) = res {
        let _ = writeln!(out, "{}", trl_format("An error has occurred during password generation:\n%1.", &[&e]));
        return 1;
    }
    app.write_rand_seed_file();
    0
}
