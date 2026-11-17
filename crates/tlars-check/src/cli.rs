//! `TLC.handleParameters` (`TLC.java:441-1340`), option by option. Pure: files are not
//! opened and directories not created here; the session does that after a successful parse.
//! Defaults are the TLC constructor's (`:216-238`): deadlock checking on (`-deadlock`
//! turns it *off*), random fp index and seed unless given.

use tlars_java::{JavaIntHashSet, parse_double, parse_int, parse_long};
use tlars_msg::Ec;
use tlars_sany::{ErrorLevel, SanyErrorCode};
use tlars_tool::{CliSpecExtras, LnCheck, PostCondition, RuntimeInvariant, TraceConstraint};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    ModelCheck,
    /// `-simulate` / `-generate` (`generate` sets `Tool.probabilistic`).
    Simulate(SimulateArgs),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SimulateArgs {
    pub(crate) generate: bool,
    /// `num=`; `None` is TLC's `Long.MAX_VALUE`.
    pub(crate) num: Option<i64>,
    pub(crate) file: Option<String>,
    /// `stats=basic` / `stats=full`.
    pub(crate) stats: Option<SimStats>,
    /// `sched=rl` / `sched=rlaction`.
    pub(crate) sched: Option<SimSched>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimStats {
    Basic,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimSched {
    Rl,
    RlAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DumpArgs {
    Plain { file: String },
    Dot { file: String, colorize: bool, action_labels: bool, snapshot: bool, constrained: bool, stuttering: bool, strict: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DebuggerArgs {
    pub(crate) port: u16,
    pub(crate) suspend: bool,
    pub(crate) halt: bool,
}

/// `-fpmem`: a fraction of the memory for fingerprints, or (deprecated) an absolute size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FpMem {
    Ratio(f64),
    Absolute(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Options {
    /// The spec file name with `.tla` stripped.
    pub(crate) spec: String,
    /// `-config`, `.cfg` stripped; `None` means the spec name.
    pub(crate) config: Option<String>,
    pub(crate) mode: Mode,
    /// `-dfid N`; `None` is BFS (`DFIDMax == -1`).
    pub(crate) dfid: Option<i32>,
    pub(crate) workers: usize,
    pub(crate) tool_mode: bool,
    pub(crate) deadlock_check: bool,
    pub(crate) cleanup: bool,
    pub(crate) continue_after_violation: bool,
    pub(crate) difftrace: bool,
    /// `-terse`: `TLCGlobals.expand = false`.
    pub(crate) terse: bool,
    pub(crate) warnings_enabled: bool,
    pub(crate) gzip: bool,
    pub(crate) view: bool,
    pub(crate) debug: bool,
    pub(crate) suppressed: Vec<Ec>,
    pub(crate) messages_as_errors: Vec<Ec>,
    pub(crate) sany_suppressed: Vec<SanyErrorCode>,
    pub(crate) sany_messages_as_errors: Vec<SanyErrorCode>,
    pub(crate) fp_index: Option<usize>,
    pub(crate) fp_bits: Option<u32>,
    pub(crate) fp_mem: Option<FpMem>,
    pub(crate) seed: Option<i64>,
    pub(crate) aril: i64,
    /// `-depth`, simulation trace length (default 100).
    pub(crate) depth: i32,
    pub(crate) max_set_size: Option<i32>,
    pub(crate) lncheck: LnCheck,
    /// `-coverage`, minutes.
    pub(crate) coverage_minutes: Option<i32>,
    /// `-checkpoint`, minutes.
    pub(crate) checkpoint_minutes: Option<i32>,
    pub(crate) recover: Option<String>,
    pub(crate) metadir: Option<String>,
    pub(crate) user_file: Option<String>,
    pub(crate) dump: Option<DumpArgs>,
    pub(crate) debugger: Option<DebuggerArgs>,
    /// Whether the trace-exploration spec is written (after TLC's defaulting rules).
    pub(crate) generate_te_spec: bool,
    pub(crate) generate_te_spec_binary: bool,
    pub(crate) te_spec_out: Option<String>,
    pub(crate) extras: CliSpecExtras,
}

/// Why `handleParameters` returned `false` (exit 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CliExit {
    /// `printErrorMsg(msg)`: the welcome line, then 1102 with `msg`. `tool_mode` is
    /// `TLCGlobals.tool` at that moment: a `-tool` after the bad option does not frame it.
    Usage { message: String, tool_mode: bool },
    /// `-help` / `-h`.
    Help,
    /// TLC crashes with an uncaught exception while parsing (e.g. `-inv` as the last
    /// argument indexes past the array). The text is what the JVM prints to stderr.
    Crash { exception: String },
}

impl CliExit {
    fn usage(message: impl Into<String>, tool_mode: bool) -> Self {
        CliExit::Usage { message: message.into(), tool_mode }
    }
}

/// `-dumpTrace` formats (`TLC.DumpTraceFormat`): flag, module, operator, constant.
const DUMP_TRACE_FORMATS: &[(&str, &str, &str, &str)] = &[
    ("json", "_JsonTrace", "_JsonTrace", "_JsonTraceFile"),
    ("tla", "_TLAPlusCounterExample", "_TLAPlusCounterExample", "_TLAPlusCounterExampleFile"),
    ("tlc", "_TLCTrace", "_TLCTrace", "_TLCTraceFile"),
    ("tlcplain", "_TLCTracePlain", "_TLCTrace", "_TLCTraceFile"),
    ("tlcTESpec", "_TLCTESpec", "_TLCTrace", "_TLCTraceFile"),
    ("tlcaction", "_TLCActionTrace", "_TLCTrace", "_TLCTraceFile"),
    ("dot", "_DotTrace", "_DotTrace", "_DotTraceFile"),
    ("cbor", "_CBORTrace", "_CBORTrace", "_CBORTraceFile"),
];

/// `FP64.Polys.length - 1`.
const MAX_FP_INDEX: usize = 130;
/// `MultiFPSet.MAX_FPBITS`.
const MAX_FP_BITS: i32 = 30;

/// Java's `ArrayIndexOutOfBoundsException` text for `args[index]`.
fn out_of_bounds(index: usize, len: usize) -> CliExit {
    CliExit::Crash {
        exception: format!("java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {len}"),
    }
}

/// `TLAConstants.Files.TLA_EXTENSION` / `CONFIG_EXTENSION` stripping.
fn strip_suffix_once<'a>(s: &'a str, suffix: &str) -> &'a str {
    s.strip_suffix(suffix).unwrap_or(s)
}

fn get_dump_file(file: &str, suffix: &str) -> String {
    if file.ends_with(suffix) { file.to_string() } else { format!("{file}{suffix}") }
}

/// Codes of `-suppressMessages`/`-messagesAsErrors`, split into TLC and SANY ranges
/// (`classifyMessageCode`), in argument order.
struct Codes {
    tlc: Vec<i32>,
    sany: Vec<SanyErrorCode>,
}

/// `Arrays.stream(arg.split(",")).map(trim).mapToInt(parseInt).forEach(classify)`.
/// `reject_sany_errors`: `-suppressMessages` refuses SANY error-level codes.
fn parse_codes(arg: &str, reject_sany_errors: bool) -> Result<Codes, String> {
    let mut codes = Codes { tlc: Vec::new(), sany: Vec::new() };
    // Java's `String.split` drops trailing empty strings.
    let mut parts: Vec<&str> = arg.split(',').collect();
    while parts.len() > 1 && parts.last() == Some(&"") {
        parts.pop();
    }
    for part in parts {
        let code = parse_int(part.trim()).map_err(|e| e.message())?;
        if Ec::from_code(code).is_some() {
            codes.tlc.push(code);
        } else if let Some(sany) = SanyErrorCode::from_standard_value(code) {
            if reject_sany_errors && sany.level == ErrorLevel::Error {
                return Err(format!("code {code} is an error and cannot be suppressed."));
            }
            codes.sany.push(sany);
        } else {
            return Err(format!("unknown message code: {code}"));
        }
    }
    Ok(codes)
}

/// `TLC.handleParameters`. `cores` answers `-workers auto`.
pub(crate) fn parse(args: &[String], cores: usize) -> Result<Options, CliExit> {
    let mut o = Options {
        spec: String::new(),
        config: None,
        mode: Mode::ModelCheck,
        dfid: None,
        workers: 1,
        tool_mode: false,
        deadlock_check: true,
        cleanup: false,
        continue_after_violation: false,
        difftrace: false,
        terse: false,
        warnings_enabled: true,
        gzip: false,
        view: false,
        debug: false,
        suppressed: Vec::new(),
        messages_as_errors: Vec::new(),
        sany_suppressed: Vec::new(),
        sany_messages_as_errors: Vec::new(),
        fp_index: None,
        fp_bits: None,
        fp_mem: None,
        seed: None,
        aril: 0,
        depth: 100,
        max_set_size: None,
        lncheck: LnCheck::Default,
        coverage_minutes: None,
        checkpoint_minutes: None,
        recover: None,
        metadir: None,
        user_file: None,
        dump: None,
        debugger: None,
        generate_te_spec: true,
        generate_te_spec_binary: true,
        te_spec_out: None,
        extras: CliSpecExtras::default(),
    };
    let mut main_file: Option<String> = None;
    let mut force_te_spec = false;
    let mut tlc_suppressed: Vec<i32> = Vec::new();
    let mut tlc_as_errors: Vec<i32> = Vec::new();

    let n = args.len();
    let mut i = 0;
    // The argument after an option, or the message TLC prints when it is missing.
    macro_rules! next_arg {
        ($missing:expr) => {{
            if i < n {
                i += 1;
                args[i - 1].as_str()
            } else {
                return Err(CliExit::usage($missing, o.tool_mode));
            }
        }};
    }
    // `args[index++]` without a bounds check, as TLC does in a few places.
    macro_rules! unchecked_arg {
        () => {{
            if i >= n {
                return Err(out_of_bounds(i, n));
            }
            i += 1;
            args[i - 1].as_str()
        }};
    }

    while i < n {
        let arg = args[i].as_str();
        i += 1;
        match arg {
            "-simulate" | "-generate" => {
                let mut sim = SimulateArgs { generate: arg == "-generate", ..SimulateArgs::default() };
                if i < n && ["stats=", "file=", "num=", "sched"].iter().any(|k| args[i].contains(k)) {
                    for part in args[i].split(',') {
                        if let Some(num) = part.strip_prefix("num=") {
                            // `Long.parseLong(arg.replace("num=", ""))`: a bad number crashes TLC.
                            sim.num = Some(parse_long(&part.replace("num=", "")).map_err(|e| CliExit::Crash {
                                exception: format!("java.lang.NumberFormatException: {}", e.message()),
                            })?);
                            let _ = num;
                        } else if part.starts_with("file=") {
                            sim.file = Some(part.replace("file=", ""));
                        } else if part == "stats=basic" {
                            sim.stats = Some(SimStats::Basic);
                        } else if part == "stats=full" {
                            sim.stats = Some(SimStats::Full);
                        } else if part == "sched=rl" {
                            sim.sched = Some(SimSched::Rl);
                        } else if part == "sched=rlaction" {
                            sim.sched = Some(SimSched::RlAction);
                        }
                    }
                    i += 1;
                    if sim.num.is_none_or(|v| v == i64::MAX) && sim.file.is_some() {
                        return Err(CliExit::usage(
                            "Error: You need to specify a 'num' argument when using the 'file' argument, for example: '\"num=5,file=test.txt\"'.",
                            o.tool_mode,
                        ));
                    }
                }
                o.mode = Mode::Simulate(sim);
            }
            "-modelcheck" => {}
            "-difftrace" => o.difftrace = true,
            "-deadlock" => o.deadlock_check = false,
            "-cleanup" => o.cleanup = true,
            "-nowarning" => o.warnings_enabled = false,
            "-suppressMessages" => {
                let list = next_arg!("Error: -suppressMessages requires a comma-separated list of message codes.");
                let codes = parse_codes(list, true).map_err(|e| CliExit::usage(format!("Error: {e}"), o.tool_mode))?;
                tlc_suppressed.extend(codes.tlc);
                o.sany_suppressed.extend(codes.sany);
            }
            "-messagesAsErrors" => {
                let list = next_arg!("Error: -messagesAsErrors requires a comma-separated list of message codes.");
                let codes = parse_codes(list, false).map_err(|e| CliExit::usage(format!("Error: {e}"), o.tool_mode))?;
                tlc_as_errors.extend(codes.tlc);
                o.sany_messages_as_errors.extend(codes.sany);
            }
            "-gzip" => o.gzip = true,
            "-terse" => o.terse = true,
            "-continue" => o.continue_after_violation = true,
            "-view" => o.view = true,
            "-debug" => o.debug = true,
            "-inv" => {
                let expr = unchecked_arg!();
                let mut modules: Vec<String> = Vec::new();
                for (pos, _) in expr.match_indices("INSTANCE") {
                    let rest = &expr[pos + "INSTANCE".len()..];
                    let trimmed = rest.trim_start();
                    if trimmed.len() == rest.len() {
                        continue; // `INSTANCE\s+` requires whitespace
                    }
                    let name: String = trimmed.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if !name.is_empty() && !modules.contains(&name) {
                        modules.push(name);
                    }
                }
                o.extras.invariants.push(RuntimeInvariant { modules, expr: expr.to_string() });
            }
            "-invlevel" => {
                let level = unchecked_arg!();
                if parse_int(level).is_err() {
                    return Err(CliExit::usage(
                        format!("Error: An integer for -invlevel required. But encountered {level}"),
                        o.tool_mode,
                    ));
                }
                o.extras.invariants.push(RuntimeInvariant {
                    modules: vec!["TLC".into(), "Naturals".into()],
                    expr: format!("LET _T == INSTANCE TLC _N == INSTANCE Naturals IN _N!<(_T!TLCGet(\"level\"), {level})"),
                });
            }
            "-debugger" => {
                let mut dbg = DebuggerArgs { port: 4712, suspend: true, halt: true };
                if i < n && ["port=", "nosuspend", "nohalt", "suspend", "halt"].iter().any(|k| args[i].contains(k)) {
                    let lower = args[i].to_lowercase();
                    dbg.suspend = !lower.contains("nosuspend");
                    dbg.halt = !lower.contains("nohalt");
                    // `.*port=([0-9]{1,5}).*`: the last `port=` with 1 to 5 digits.
                    if let Some(port) = args[i].rmatch_indices("port=").find_map(|(p, _)| {
                        let digits: String = args[i][p + 5..].chars().take_while(char::is_ascii_digit).take(5).collect();
                        (!digits.is_empty()).then_some(digits)
                    }) {
                        dbg.port = port.parse().unwrap_or(u16::MAX);
                    }
                    i += 1;
                }
                o.debugger = Some(dbg);
            }
            "-tool" => o.tool_mode = true,
            "-generateSpecTE" => {
                force_te_spec = true;
                if i < n && args[i] == "nomonolith" {
                    i += 1;
                }
            }
            "-noGenerateSpecTEBin" => o.generate_te_spec_binary = false,
            _ if arg == "-noGenerateSpecTE" || arg.eq_ignore_ascii_case("-noTE") => o.generate_te_spec = false,
            _ if arg.eq_ignore_ascii_case("-noTEBin") => o.generate_te_spec_binary = false,
            "-teSpecOutDir" => {
                o.te_spec_out = Some(next_arg!("Error: expected a path for -teSpecOutDir option.").to_string());
            }
            "-help" | "-h" => return Err(CliExit::Help),
            "-lncheck" => {
                let strategy = next_arg!("Error: expect a strategy such as final for -lncheck option.").to_lowercase();
                // `TLCGlobals.lnCheck` is the lowercased string; it is interpreted later.
                o.lncheck = match strategy.as_str() {
                    "final" => LnCheck::Final,
                    "seqfinal" => LnCheck::SeqFinal,
                    "off" => LnCheck::Off,
                    _ => LnCheck::Default,
                };
            }
            "-config" => {
                let config = next_arg!("Error: expect a file name for -config option.");
                o.config = Some(strip_suffix_once(config, ".cfg").to_string());
            }
            "-dump" => {
                let first = unchecked_arg!();
                if let Some(class) = first.strip_prefix("class,") {
                    // tlars cannot load Java classes: TLC's message for a failed instantiation.
                    return Err(CliExit::usage(
                        format!("Trying to instantiate a custom IStateWriter implementation {}.", class.replace("class,", "")),
                        o.tool_mode,
                    ));
                } else if i < n && first.starts_with("dot") {
                    let flags = first.to_lowercase();
                    let file = get_dump_file(&args[i], ".dot");
                    i += 1;
                    o.dump = Some(DumpArgs::Dot {
                        file,
                        colorize: flags.contains("colorize"),
                        action_labels: flags.contains("actionlabels"),
                        snapshot: flags.contains("snapshot"),
                        constrained: flags.contains("constrained"),
                        stuttering: flags.contains("stuttering"),
                        strict: flags.contains("strict"),
                    });
                } else {
                    o.dump = Some(DumpArgs::Plain { file: get_dump_file(first, ".dump") });
                }
            }
            _ if arg.eq_ignore_ascii_case("-loadTrace") => {
                if i + 1 >= n {
                    return Err(CliExit::usage("Error: A format and a file name for loading traces required.", o.tool_mode));
                }
                let fmt = &args[i];
                let file = args[i + 1].clone();
                i += 2;
                let (module, operator, constant, view) = if fmt.eq_ignore_ascii_case("tlc") {
                    ("_TLCTrace", "_TLCTraceConstraint", "_TLCTraceInputFile", "_TLCTraceView")
                } else if fmt.eq_ignore_ascii_case("json") {
                    ("_JsonTrace", "_JsonTraceConstraint", "_JsonTraceInputFile", "_JsonTraceView")
                } else {
                    return Err(CliExit::usage(format!("Error: Unknown format {fmt} given to -loadTrace."), o.tool_mode));
                };
                o.extras.constraints.push(TraceConstraint {
                    module: module.into(),
                    operator: operator.into(),
                    constant: (constant.into(), file),
                });
                o.extras.view.get_or_insert_with(|| (module.into(), view.into()));
            }
            _ if arg.eq_ignore_ascii_case("-dumpTrace") => {
                if i + 1 >= n {
                    return Err(CliExit::usage("Error: A format and a file name for dumping traces required.", o.tool_mode));
                }
                let fmt = &args[i];
                let Some(&(_, module, operator, constant)) =
                    DUMP_TRACE_FORMATS.iter().find(|(flag, ..)| flag.eq_ignore_ascii_case(fmt))
                else {
                    return Err(CliExit::usage(format!("Error: Unknown format {fmt} given to -dumpTrace."), o.tool_mode));
                };
                o.extras.postconditions.push(PostCondition {
                    module: module.into(),
                    operator: operator.into(),
                    constant: Some((constant.into(), args[i + 1].clone())),
                });
                i += 2;
            }
            _ if arg.eq_ignore_ascii_case("-postCondition") => {
                let spec = next_arg!("Error: Module!Operator for postCondition required.");
                let fail = |m: String| Err(CliExit::usage(m, o.tool_mode));
                if !spec.contains('!') {
                    return fail(format!("Module!Operator for postCondition required. Encountered: {spec}"));
                }
                let parts = java_split(spec, '!');
                if parts.len() != 2 {
                    return fail(format!("Module!Operator for postCondition must be of the form module!operator. Encountered: {spec}"));
                }
                let (module, def) = (parts[0], parts[1]);
                if module.is_empty() {
                    return fail(format!("Module name for postCondition required. Encountered: {spec}"));
                }
                if module.contains('.') {
                    return fail(format!("Module name for postCondition must not contain a dot. Encountered: {module}"));
                }
                if def.is_empty() {
                    return fail(format!("Operator name for postCondition required. Encountered: {spec}"));
                }
                if def.contains('.') {
                    return fail(format!("Operator name for postCondition must not contain a dot. Encountered: {def}"));
                }
                o.extras.postconditions.push(PostCondition { module: module.into(), operator: def.into(), constant: None });
            }
            "-coverage" => {
                let v = next_arg!("Error: coverage report interval required.");
                let minutes = parse_int(v).map_err(|_| {
                    CliExit::usage(format!("Error: An integer for coverage report interval required. But encountered {v}"), o.tool_mode)
                })?;
                if minutes.wrapping_mul(60 * 1000) < 0 {
                    return Err(CliExit::usage("Error: expect a nonnegative integer for -coverage option.", o.tool_mode));
                }
                o.coverage_minutes = Some(minutes);
            }
            "-checkpoint" => {
                let v = next_arg!("Error: checkpoint interval required.");
                let minutes = parse_int(v).map_err(|_| {
                    CliExit::usage(format!("Error: An integer for checkpoint interval is required. But encountered {v}"), o.tool_mode)
                })?;
                if minutes.wrapping_mul(1000 * 60) < 0 {
                    return Err(CliExit::usage("Error: expect a nonnegative integer for -checkpoint option.", o.tool_mode));
                }
                o.checkpoint_minutes = Some(minutes);
            }
            "-depth" => {
                let v = next_arg!("Error: trace length required.");
                o.depth = parse_int(v).map_err(|_| {
                    CliExit::usage(format!("Error: An integer for trace length required. But encountered {v}"), o.tool_mode)
                })?;
            }
            "-seed" => {
                let v = next_arg!("Error: seed required.");
                o.seed = Some(parse_long(v).map_err(|_| {
                    CliExit::usage(format!("Error: An integer for seed required. But encountered {v}"), o.tool_mode)
                })?);
            }
            "-aril" => {
                let v = next_arg!("Error: aril required.");
                o.aril = parse_long(v).map_err(|_| {
                    CliExit::usage(format!("Error: An integer for aril required. But encountered {v}"), o.tool_mode)
                })?;
            }
            "-maxSetSize" => {
                let v = next_arg!("Error: maxSetSize required.");
                let bound = parse_int(v).map_err(|_| {
                    CliExit::usage(format!("Error: An integer for maxSetSize required. But encountered {v}"), o.tool_mode)
                })?;
                if bound < 1 {
                    return Err(CliExit::usage(
                        format!("Error: Value in interval [0, {}] for maxSetSize required. But encountered {v}", i32::MAX),
                        o.tool_mode,
                    ));
                }
                o.max_set_size = Some(bound);
            }
            "-recover" => {
                let dir = next_arg!("Error: need to specify the metadata directory for recovery.");
                o.recover = Some(format!("{dir}/"));
            }
            "-metadir" => {
                let dir = next_arg!("Error: need to specify the metadata directory.");
                o.metadir = Some(format!("{dir}/"));
            }
            "-userFile" => {
                o.user_file = Some(next_arg!("Error: need to specify the full qualified file.").to_string());
            }
            "-workers" => {
                let v = next_arg!("Error: expect an integer or 'auto' for -workers option.");
                let num = if v.trim().to_lowercase() == "auto" {
                    i32::try_from(cores).unwrap_or(i32::MAX)
                } else {
                    parse_int(v).map_err(|_| {
                        CliExit::usage(format!("Error: worker number or 'auto' required. But encountered {v}"), o.tool_mode)
                    })?
                };
                if num < 1 {
                    return Err(CliExit::usage("Error: at least one worker required.", o.tool_mode));
                }
                o.workers = num as usize;
            }
            "-dfid" => {
                let v = next_arg!("Error: expect a nonnegative integer for -dfid option.");
                let depth = parse_int(v).map_err(|_| {
                    CliExit::usage(format!("Error: expect a nonnegative integer for -dfid option. But encountered {v}"), o.tool_mode)
                })?;
                if depth < 0 {
                    return Err(CliExit::usage("Error: expect a nonnegative integer for -dfid option.", o.tool_mode));
                }
                o.dfid = Some(depth);
            }
            "-fp" => {
                let v = next_arg!("Error: expect an integer for -fp option.");
                let index = parse_int(v).map_err(|_| {
                    CliExit::usage(format!("Error: A number for -fp is required. But encountered {v}"), o.tool_mode)
                })?;
                if index < 0 || index as usize > MAX_FP_INDEX {
                    return Err(CliExit::usage(
                        format!("Error: The number for -fp must be between 0 and {MAX_FP_INDEX} (inclusive)."),
                        o.tool_mode,
                    ));
                }
                o.fp_index = Some(index as usize);
            }
            "-fpmem" => {
                let v = next_arg!("Error: fpset memory size required.");
                let bad = || {
                    CliExit::usage(
                        format!("Error: An positive integer or a fraction for fpset memory size/percentage required. But encountered {v}"),
                        o.tool_mode,
                    )
                };
                let size = parse_double(v).map_err(|_| bad())?;
                if size < 0.0 {
                    return Err(bad());
                }
                // The deprecation notice for an absolute size is printed by the session,
                // after the welcome line, in TLC's position (`ToolIO.out` during parsing).
                o.fp_mem = Some(if size > 1.0 { FpMem::Absolute(size as u64) } else { FpMem::Ratio(size) });
            }
            "-fpbits" => {
                let v = next_arg!("Error: fpbits required.");
                let bits = parse_int(v)
                    .map_err(|_| CliExit::usage(format!("Error: An integer for fpbits required. But encountered {v}"), o.tool_mode))?;
                if !(0..=MAX_FP_BITS).contains(&bits) {
                    return Err(CliExit::usage(
                        format!("Error: Value in interval [0, 30] for fpbits required. But encountered {v}"),
                        o.tool_mode,
                    ));
                }
                o.fp_bits = Some(bits as u32);
            }
            _ => {
                if arg.starts_with('-') {
                    return Err(CliExit::usage(format!("Error: unrecognized option: {arg}"), o.tool_mode));
                }
                if let Some(first) = &main_file {
                    return Err(CliExit::usage(format!("Error: more than one input files: {first} and {arg}"), o.tool_mode));
                }
                main_file = Some(strip_suffix_once(arg, ".tla").to_string());
            }
        }
    }

    let has_suppressed = !o.sany_suppressed.is_empty() || !tlc_suppressed.is_empty();
    if !o.warnings_enabled && has_suppressed {
        return Err(CliExit::usage(
            "Error: -nowarning already suppresses all warnings, making -suppressMessages redundant. Remove -nowarning to suppress only the specified messages, or remove -suppressMessages to suppress all warnings.",
            o.tool_mode,
        ));
    }
    // `checkForMessageCodeOverlap`: `new HashSet<>(suppressed).retainAll(asErrors)`.
    let mut suppressed_set = JavaIntHashSet::new();
    tlc_suppressed.iter().for_each(|&c| _ = suppressed_set.add(c));
    let mut as_errors_set = JavaIntHashSet::new();
    tlc_as_errors.iter().for_each(|&c| _ = as_errors_set.add(c));
    let mut overlap = JavaIntHashSet::copy_of(&suppressed_set);
    overlap.retain(|c| as_errors_set.contains(c));
    if !overlap.is_empty() {
        return Err(CliExit::usage(
            format!("Error: The following codes were set to both -suppressMessages and -messagesAsErrors: {}", overlap.to_java_string()),
            o.tool_mode,
        ));
    }
    // SANY codes are enum constants in a `HashSet`, whose order follows identity hash
    // codes: TLC's own order is not reproducible. Names are listed in argument order.
    let sany_overlap: Vec<&str> =
        o.sany_suppressed.iter().filter(|c| o.sany_messages_as_errors.contains(c)).map(|c| c.name).collect();
    if !sany_overlap.is_empty() {
        return Err(CliExit::usage(
            format!("Error: The following codes were set to both -suppressMessages and -messagesAsErrors: [{}]", sany_overlap.join(", ")),
            o.tool_mode,
        ));
    }
    o.suppressed = tlc_suppressed.iter().filter_map(|&c| Ec::from_code(c)).collect();
    o.messages_as_errors = tlc_as_errors.iter().filter_map(|&c| Ec::from_code(c)).collect();

    let Some(spec) = main_file else {
        return Err(CliExit::usage("Error: Missing input TLA+ module.", o.tool_mode));
    };
    o.generate_te_spec = force_te_spec
        || (o.generate_te_spec && !o.tool_mode && !o.continue_after_violation && !is_te_spec_file(&spec));
    // The trace spec's binary trace (`_TLCTrace!_TLCTraceSilent` postcondition) needs the
    // run's start time for its file name, so the session adds it.
    o.spec = spec;
    Ok(o)
}

/// `TraceExplorationSpec.isTESpecFile`: the file name matches `^.*_TTrace.*(.tla)?$`.
fn is_te_spec_file(spec: &str) -> bool {
    let file_name = spec.rsplit(['/', '\\']).next().unwrap_or(spec);
    file_name.contains("_TTrace")
}

/// `String.split(char)`: trailing empty strings are removed.
fn java_split(s: &str, sep: char) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(sep).collect();
    while parts.len() > 1 && parts.last() == Some(&"") {
        parts.pop();
    }
    if parts == [""] && !s.is_empty() {
        parts.clear();
    }
    parts
}
