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
    if session.options.debug {
        todo!("M5: -debug (DebugPrinter output from handleParameters on, and exception stack traces)");
    }
    // `printWelcome`, the last step of `handleParameters` (`TLC.java:1318`).
    out.message(Ec::TLC_VERSION, &[crate::VERSION]);
    if session.options.recover.is_some() {
        todo!("M3: -recover (UniqueString.internTbl.recover before FP64.Init, TLC.java:1349-1354)");
    }
    if session.options.extras != tlars_tool::CliSpecExtras::default() {
        // `checkDumpTraceModules` runs here, before the banner (`TLC.java:1356`).
        todo!("M3: -dumpTrace, -postCondition, -inv and -loadTrace");
    }
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
    // The trace-exploration spec is written only for a recorded error trace
    // (`TraceExplorationSpec.java:73-76`), and no trace is recorded before M3's search.
    Exit::Returned(result)
}

/// `getModelCheckingRuntime` (`TLC.java:1560-1596`), in `LinkedHashMap` order. The JVM's
/// entries describe tlars (DESIGN.md); the FPSet and queue names follow TLC's properties.
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
        simple_class_name(session.property("tlc2.tool.fp.FPSet.impl").unwrap_or("tlc2.tool.fp.MSBDiskFPSet")),
        session.property("tlc2.tool.queue.IStateQueue").unwrap_or("DiskStateQueue").to_string(),
    ]
}

/// `fpSetConfig.getImplementation()` after its last `.`.
fn simple_class_name(class: &str) -> String {
    class.rsplit('.').next().unwrap_or(class).to_string()
}

/// The `RunMode.MODEL_CHECK` branch of `process()`: `new FastTool(...)`, then
/// `ModelChecker.modelCheck()`. `Err` carries what the `catch` block prints.
fn model_check(session: &Session<'_>) -> Result<Ec, Exit> {
    let out = &session.reporter;
    let (user_dir, main_file) = main_file(session);
    let library_paths = library_paths(session, &main_file);
    let config = read_config(session, user_dir.as_deref(), &library_paths)?;
    if session.options.tool_mode {
        out.message(Ec::TLC_SANY_START, &[]);
    }
    // `DelayedPrintStream` (`SpecProcessor.java:420`): with 2220 suppressed, SANY's console
    // output is held back, and dropped unless SANY throws.
    let delayed = session.options.suppressed.contains(&Ec::TLC_SANY_START);
    let mut sany_out = SanyLines { out, stderr: &session.stderr, held: delayed.then(Vec::new) };
    let loaded = tlars_sany::load(
        LoadRequest {
            root_file: &main_file,
            user_dir: user_dir.as_deref(),
            working_dir: &working_dir(session),
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
        Err(SanyFailed::FrontEnd) => todo!("M5: TLC_PARSING_FAILED2 with the FrontEndException, after releasing held SANY output"),
        Err(SanyFailed::Parse { errors } | SanyFailed::Semantic { errors }) => {
            let errors = errors.iter().map(String::as_str).collect::<Vec<_>>();
            return Err(Exit::Returned(out.runtime_exception(Ec::TLC_PARSING_FAILED, &errors)));
        }
        // `Assert.fail(TLC_PARSING_FAILED)` without parameters (`SpecProcessor.java:458`).
        Err(SanyFailed::Exit(_)) => return Err(Exit::Returned(print_failure(out, Ec::TLC_PARSING_FAILED))),
    };
    let globals = globals(session);
    let tool = match tlars_tool::build_tool(ToolInput { graph, config, globals, extras: session.options.extras.clone(), out }) {
        Ok(tool) => tool,
        Err(SpecError::Config { ec, params }) => {
            let params = params.iter().map(String::as_str).collect::<Vec<_>>();
            return Err(Exit::Returned(out.runtime_exception(ec, &params)));
        }
        Err(SpecError::Failure { ec }) => return Err(Exit::Returned(print_failure(out, ec))),
        Err(SpecError::Unexpected { class, message }) => {
            let body = tlars_msg::unexpected_exception(class, &message);
            return Err(Exit::Returned(out.error(Ec::GENERAL, &[&body])));
        }
        Err(SpecError::UnsupportedJavaOverride { .. }) => todo!("M5: user Java overrides"),
    };
    Ok(crate::bfs::model_check(&tool, session))
}

/// `printTLCRuntimeException` for `Assert.fail(ec)`: with no parameters it falls back to
/// `printError(ec, exception)`, which prints the exception's message as the parameter and is
/// not suppressible (`MP.java:1852-1855`, `1797-1803`).
fn print_failure(out: &dyn Report, ec: Ec) -> Ec {
    let message = out.text(ec, &[]);
    out.error(ec, &[&message])
}

/// The directory a resolver without a user directory searches: the `user.dir` property
/// (`SimpleFilenameToStream.java:112-120`), which defaults to the working directory.
fn working_dir(session: &Session<'_>) -> PathBuf {
    session.property("user.dir").map_or_else(|| session.cwd.clone(), |d| session.cwd.join(d))
}

/// `Path.of(s).toString()` on Unix: repeated and trailing separators go, `.` and `..` stay.
fn path_of(s: &str) -> PathBuf {
    let segments: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    let joined = segments.join("/");
    PathBuf::from(if s.starts_with('/') { format!("/{joined}") } else { joined })
}

/// `TLC.main`'s resolver (`TLC.java:343-354`): a main file with a directory part searches
/// that directory after the user directory, in place of `-DTLA-Library`. Paths stay as
/// written (`Path.of`), relative ones being relative to the working directory.
fn library_paths(session: &Session<'_>, main_file: &str) -> Vec<PathBuf> {
    // `FileUtil.parseDirname`: everything up to the last separator.
    match main_file.rfind(std::path::MAIN_SEPARATOR) {
        Some(i) => vec![path_of(&main_file[..=i])],
        None => session.property("TLA-Library").map_or_else(Vec::new, |p| java_split(p, PATH_SEPARATOR).map(path_of).collect()),
    }
}

/// `File.pathSeparator`.
const PATH_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// `String.split(sep)` (`SimpleFilenameToStream.parsePaths`): trailing empty entries are
/// dropped, inner and leading ones kept (an empty path is the working directory).
fn java_split(s: &str, sep: char) -> impl Iterator<Item = &str> {
    let mut parts: Vec<&str> = s.split(sep).collect();
    if parts.len() > 1 {
        while parts.last() == Some(&"") {
            parts.pop();
        }
    }
    parts.into_iter()
}

/// The end of `handleParameters` (`TLC.java:1236-1242`): an absolute spec path becomes the
/// user directory and its file name the main file. Otherwise no user directory is set and
/// modules are found from the working directory (the invocation's, not this process's).
fn main_file(session: &Session<'_>) -> (Option<PathBuf>, String) {
    let spec = Path::new(&session.options.spec);
    if spec.is_absolute() {
        let name = spec.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (spec.parent().map(Path::to_path_buf), name)
    } else {
        (None, session.options.spec.clone())
    }
}

/// `new ModelConfig(...).parse()` (`ModelConfig.java:171-180`): `resolver.resolve(name,
/// false)` looks in the user directory, then the library paths. The standard-module
/// locations it tries next hold no `.cfg`.
fn read_config(session: &Session<'_>, user_dir: Option<&Path>, library_paths: &[PathBuf]) -> Result<tlars_sany::cfg::ModelConfig, Exit> {
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
    // `resolve(name, false)` (`SimpleFilenameToStream.java:254-264`) keeps the part before a
    // newline; the error messages keep the whole name.
    let lookup = match name.find('\n') {
        Some(n) => {
            out.raw(&format!("*** Warning: module name '{name}' contained NEWLINE; Only the part before NEWLINE is considered.\n"));
            &name[..n]
        }
        None => name.as_str(),
    };
    let base_dir = working_dir(session);
    let dirs = std::iter::once(user_dir.unwrap_or(Path::new(""))).chain(library_paths.iter().map(PathBuf::as_path));
    let found = dirs.map(|d| base_dir.join(d).join(lookup)).find(|p| p.exists());
    let text = match found.map(|p| open_like_file_input_stream(&p).map(read_until_failure)) {
        Some(Ok(text)) => text,
        // `FileUtil.newFIS` (`FileUtil.java:427-440`): the file exists but cannot be opened.
        Some(Err(_)) => {
            out.raw("***Internal error: Unable to create FileInputStream\n");
            out.error(Ec::CFG_ERROR_READING_FILE, &[&name, "File not found."]);
            return Err(Exit::System(Ec::CFG_ERROR_READING_FILE.exit_status()));
        }
        None => {
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

/// `new FileInputStream(file)`, which also refuses a directory (Unix `open` accepts one).
fn open_like_file_input_stream(path: &Path) -> std::io::Result<std::fs::File> {
    if path.is_dir() {
        return Err(std::io::Error::from(std::io::ErrorKind::IsADirectory));
    }
    std::fs::File::open(path)
}

/// What the token manager sees of an opened cfg: a read failure ends the input like EOF
/// (`TLAplusParserTokenManager.java:5167-5176`), keeping what was read before it.
fn read_until_failure(mut file: std::fs::File) -> Vec<u8> {
    use std::io::Read;
    let mut text = Vec::new();
    let _ = file.read_to_end(&mut text);
    text
}

fn globals(session: &Session<'_>) -> TlcGlobals {
    let o = &session.options;
    TlcGlobals {
        tool_mode: o.tool_mode,
        workers: o.workers,
        check_deadlock: o.deadlock_check,
        continue_after_violation: o.continue_after_violation,
        difftrace: o.difftrace,
        use_view: o.view,
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
    /// `DelayedPrintStream`'s buffer, when 2220 is suppressed.
    held: Option<Vec<String>>,
}

impl SanyOut for SanyLines<'_, '_> {
    fn log_line(&mut self, text: &str) {
        match self.held.as_mut() {
            Some(held) => held.push(text.to_string()),
            None => self.out.raw(&format!("{text}\n")),
        }
    }
    /// Direct `ToolIO.out` prints bypass the `DelayedPrintStream`.
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
