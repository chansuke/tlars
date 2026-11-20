//! `TLC.process()` (`TLC.java:1337-1474`) + `Spec`/`SpecProcessor` sequencing.

use crate::cli::Mode;
use crate::session::Session;
use std::path::{Path, PathBuf};
use tlars_msg::{Ec, ExitStatus, Report};
use tlars_sany::{LoadRequest, SanyFailed, SanyOut};
use tlars_tool::{SpecError, TlcGlobals, ToolInput};

/// How the run ended: `process()` returned an error code, or TLC called `System.exit`
/// directly (skipping `process()`'s `finally`).
pub(crate) enum Exit {
    Returned(Ec),
    System(ExitStatus),
}

/// Order (each step's messages are TLC's):
///  1. `TLC2 Version` (2262), mode banner (2187 / 2188 / 2271) with the ordered parameter map.
///  2. cfg: `tlars_sany::cfg::parse` into the session interner (before SANY).
///  3. 2220, `tlars_sany::load` with SANY's own lines, 2219; failure → TLC_PARSING_FAILED (150).
///  4. `tlars_tool::build_tool` (TLC_STARTING, bindings, folding, splitting); config errors → 151.
///  5. Mode: `bfs::ModelChecker` / `dfid` / `simulate`, which run assumptions (exit 10) first.
///  6. `finally`: 2186 `Finished in ...`, trace-exploration spec (unless -tool/-continue/TE).
pub(crate) fn process(session: &Session<'_>) -> Exit {
    let out = &session.reporter;
    // `printWelcome`, the last step of `handleParameters` (`TLC.java:1318`).
    out.message(Ec::TLC_VERSION, &[crate::VERSION]);
    if session.options.mode != Mode::ModelCheck || session.options.dfid.is_some() {
        todo!("M5: simulation and DFID runs");
    }
    out.message(Ec::TLC_MODE_MC, &banner_parameters(session).iter().map(String::as_str).collect::<Vec<_>>());
    let result = match model_check(session) {
        Ok(ec) => ec,
        Err(Exit::System(status)) => return Exit::System(status),
        Err(Exit::Returned(ec)) => ec,
    };
    let runtime = session.now_millis() - session.start_millis;
    let text = if session.options.tool_mode || session.property_is_true("tlc2.TLC.asMilliSeconds") {
        format!("{runtime}ms")
    } else {
        human_readable_runtime(runtime)
    };
    out.message(Ec::TLC_FINISHED, &[&text]);
    if session.options.generate_te_spec && result != Ec::NO_ERROR {
        todo!("M3: the trace-exploration spec");
    }
    Exit::Returned(result)
}

/// `getModelCheckingRuntime` (`TLC.java:1560-1596`), in `LinkedHashMap` order. The JVM's
/// entries describe tlars; the FPSet and queue names are the TLC defaults tlars mirrors.
fn banner_parameters(session: &Session<'_>) -> Vec<String> {
    let workers = session.options.workers;
    let host = &session.env.host;
    vec![
        workers.to_string(),
        if workers == 1 { String::new() } else { "s".to_string() },
        host.cores.to_string(),
        host.os_name.clone(),
        host.os_version.clone(),
        host.os_arch.clone(),
        "tlars".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
        "64bit".to_string(),
        "0".to_string(),
        "0".to_string(),
        session.seed.to_string(),
        session.fp_index.to_string(),
        session.env.pid.to_string(),
        "MSBDiskFPSet".to_string(),
        "DiskStateQueue".to_string(),
    ]
}

/// The `RunMode.MODEL_CHECK` branch of `process()`: `new FastTool(...)`, then
/// `ModelChecker.modelCheck()`. `Err` carries what the `catch` block prints.
fn model_check(session: &Session<'_>) -> Result<Ec, Exit> {
    let out = &session.reporter;
    let (user_dir, main_file) = main_file(session);
    let config = read_config(session, &user_dir)?;
    if session.options.tool_mode {
        out.message(Ec::TLC_SANY_START, &[]);
    }
    let library_paths: Vec<PathBuf> = session.property("TLA-Library").map_or_else(Vec::new, |p| std::env::split_paths(p).collect());
    let mut sany_out = SanyLines { out, stderr: &session.stderr };
    let loaded = tlars_sany::load(
        LoadRequest {
            root_file: &main_file,
            user_dir: Some(&user_dir),
            library_paths: &library_paths,
            interner: session.interner.clone(),
            extra_root_extends: &[],
            suppressed_codes: &session.options.sany_suppressed,
            messages_as_errors: &session.options.sany_messages_as_errors,
        },
        &mut sany_out,
    );
    if session.options.tool_mode {
        out.message(Ec::TLC_SANY_END, &[]);
    }
    out.message(Ec::TLC_STARTING, &[]);
    let graph = match loaded {
        Ok(graph) => graph,
        Err(SanyFailed::FrontEnd) => todo!("M5: TLC_PARSING_FAILED2 with the FrontEndException"),
        Err(SanyFailed::Parse { errors } | SanyFailed::Semantic { errors }) => {
            let errors = errors.iter().map(String::as_str).collect::<Vec<_>>();
            return Err(Exit::Returned(out.runtime_exception(Ec::TLC_PARSING_FAILED, &errors)));
        }
        Err(SanyFailed::Exit(_)) => return Err(Exit::Returned(out.runtime_exception(Ec::TLC_PARSING_FAILED, &[]))),
    };
    let globals = globals(session);
    let tool = match tlars_tool::build_tool(ToolInput { graph, config, globals, extras: session.options.extras.clone(), out }) {
        Ok(tool) => tool,
        Err(SpecError::Config { ec, params }) => {
            let params = params.iter().map(String::as_str).collect::<Vec<_>>();
            return Err(Exit::Returned(out.runtime_exception(ec, &params)));
        }
        Err(SpecError::UnsupportedJavaOverride { .. }) => todo!("M5: user Java overrides"),
    };
    Ok(crate::bfs::model_check(&tool, out, session.seed))
}

/// The end of `handleParameters` (`TLC.java:1236-1242`): an absolute spec path becomes the
/// user directory and its file name the main file. Otherwise modules are found from the
/// working directory, which is the invocation's, not this process's.
fn main_file(session: &Session<'_>) -> (PathBuf, String) {
    let spec = Path::new(&session.options.spec);
    if spec.is_absolute() {
        let name = spec.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (spec.parent().map_or_else(|| session.cwd.clone(), Path::to_path_buf), name)
    } else {
        (session.cwd.clone(), session.options.spec.clone())
    }
}

/// `new ModelConfig(...).parse()` (`ModelConfig.java:171-180`): the file is resolved like a
/// module, without the library fallback mattering for a `.cfg`.
fn read_config(session: &Session<'_>, user_dir: &Path) -> Result<tlars_sany::cfg::ModelConfig, Exit> {
    let out = &session.reporter;
    // `configFile = mainFile` after an absolute main file was reduced to its name.
    let base = session.options.config.clone().unwrap_or_else(|| {
        let spec = Path::new(&session.options.spec);
        if spec.is_absolute() { spec.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { session.options.spec.clone() }
    });
    if base.ends_with(".tla") {
        todo!("M5: a cfg embedded in a monolithic spec");
    }
    let name = format!("{base}.cfg");
    let text = match std::fs::read(user_dir.join(&name)) {
        Ok(text) => text,
        Err(_) => {
            out.error(Ec::CFG_ERROR_READING_FILE, &[&name, "File not found."]);
            return Err(Exit::System(Ec::CFG_ERROR_READING_FILE.exit_status()));
        }
    };
    match tlars_sany::cfg::parse(&text, &session.interner) {
        Ok(config) => Ok(config),
        Err(tlars_sany::cfg::CfgError::ConfigFile { code, params }) => {
            let params = params.iter().map(String::as_str).collect::<Vec<_>>();
            Err(Exit::Returned(out.runtime_exception(code, &params)))
        }
        Err(tlars_sany::cfg::CfgError::JavaNumberFormat { .. }) => todo!("M3: the uncaught NumberFormatException"),
    }
}

fn globals(session: &Session<'_>) -> TlcGlobals {
    let o = &session.options;
    TlcGlobals {
        tool_mode: o.tool_mode,
        workers: o.workers,
        check_deadlock: o.deadlock_check,
        continue_after_violation: o.continue_after_violation,
        difftrace: o.difftrace,
        expand: !o.terse,
        enum_bound: 2000,
        width: 80,
        warn: o.warnings_enabled,
        max_set_size: o.max_set_size.unwrap_or(1_000_000),
        lncheck: o.lncheck,
        aggressive_constant_caching: session.property_is_true("tlc2.tool.impl.SpecProcessor.aggressiveConstantCaching"),
        fp_index: session.fp_index,
        seed: session.seed,
        aril: o.aril,
        no_timestamps: session.property_is_true("tlc2.output.MP.noTimestamps"),
    }
}

/// `SimpleSanyOutput(ToolIO.out)` and `ToolIO.err`.
struct SanyLines<'s, 'a> {
    out: &'s dyn Report,
    stderr: &'s std::sync::Mutex<Box<dyn std::io::Write + Send + 'a>>,
}

impl SanyOut for SanyLines<'_, '_> {
    fn line(&mut self, text: &str) {
        self.out.raw(&format!("{text}\n"));
    }
    fn err_line(&mut self, text: &str) {
        use std::io::Write;
        let _ = writeln!(self.stderr.lock().expect("stderr lock"), "{text}");
    }
}

/// `convertRuntimeToHumanReadable` (`TLC.java:1603-1620`): `SimpleDateFormat` in UTC.
fn human_readable_runtime(runtime: i64) -> String {
    const DAY: i64 = 86_400_000;
    let hours = |r: i64| (r / 3_600_000).rem_euclid(24);
    let minutes = |r: i64| (r / 60_000).rem_euclid(60);
    let seconds = |r: i64| (r / 1000).rem_euclid(60);
    if runtime > DAY {
        let r = runtime - DAY;
        format!("{}d {:02}h", r.div_euclid(DAY) + 1, hours(r))
    } else if runtime > 3_600_000 {
        format!("{:02}h {:02}min", hours(runtime), minutes(runtime))
    } else if runtime > 60_000 {
        format!("{:02}min {:02}s", minutes(runtime), seconds(runtime))
    } else {
        format!("{:02}s", seconds(runtime))
    }
}
