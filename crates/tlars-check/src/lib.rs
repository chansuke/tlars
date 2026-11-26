//! `tlc2.TLC` and the checkers: the whole product behind one function, [`run`].
//!
//! Public surface: [`run`], [`Invocation`], [`Env`] (+ [`Clock`], [`Entropy`], [`HostInfo`]),
//! [`ExitStatus`]. The binary and the conformance harness are its only callers. Every Java
//! static TLC relies on (`MP`, `TLCGlobals`, `InternTable`, `ModelValue.mvs`, `FP64`,
//! `RandomEnumerableValues`) is a field of a per-run `Session`, so many runs can share a
//! process without leaking state into each other.
#![expect(dead_code, unused_variables, reason = "stubs for later milestones; narrowed to each stub in a later change")]

mod bfs;
mod cli;
mod dfid;
mod driver;
mod fpset;
mod liveness;
mod print;
mod queue;
mod session;
mod simulate;
mod trace;

pub use tlars_java::time::Clock;
pub use tlars_msg::ExitStatus;

use std::io::Write;
use std::path::PathBuf;

/// One TLC run.
pub struct Invocation<'a> {
    /// TLC arguments, optionally preceded by `-D<prop>=<value>` (Java system properties).
    pub args: Vec<String>,
    /// Extra system properties (the harness's per-case `-D`s).
    pub system_properties: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub stdout: Box<dyn Write + Send + 'a>,
    pub stderr: Box<dyn Write + Send + 'a>,
    /// What `System.in` reads. `TLCSet("pause", TRUE)` is its only reader: it takes one byte
    /// and goes on, so a reader at end of file resumes the run at once.
    pub stdin: Box<dyn std::io::Read + Send>,
    pub env: Env,
    /// Set by the run when a message reports a construct tlars does not implement. The
    /// conformance harness uses it to tell "blocked on a missing feature" from a real
    /// mismatch without searching the output.
    pub unimplemented: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Invocation<'static> {
    /// The real process: std streams, system clock, OS entropy.
    pub fn from_process(args: Vec<String>) -> Self {
        Invocation {
            args,
            system_properties: Vec::new(),
            cwd: std::env::current_dir().unwrap_or_default(),
            stdout: Box::new(std::io::stdout()),
            stderr: Box::new(std::io::stderr()),
            stdin: Box::new(std::io::stdin()),
            env: Env::process(),
            unimplemented: None,
        }
    }
}

/// Everything volatile TLC reads from the JVM/OS. Hermetic runs make output a pure
/// function of (args, properties, files).
pub struct Env {
    pub clock: Clock,
    pub pid: u32,
    /// `java.io.tmpdir`, where SANY copies the standard modules it parses.
    pub temp_dir: PathBuf,
    /// Source of TLC's random defaults (`-fp` index, `-seed`) when not given.
    pub entropy: Entropy,
    /// Worker thread stack (TLC conformance recordings ran with `-Xss2m`).
    pub worker_stack_bytes: usize,
    pub host: HostInfo,
    /// `TLCGlobals.Version.installLocation()`, the jar TLC runs from: where this tool lives.
    /// `TLCGet("config").install` is its only reader.
    pub install_location: String,
    /// The locale the JVM's `setlocale(LC_ALL, "")` would select, which decides the C
    /// library text inside a `FileNotFoundException` message.
    pub locale: tlars_java::Locale,
}

pub enum Entropy {
    /// `std::collections::hash_map::RandomState` as a std-only entropy source.
    Os,
    Fixed(u64),
}

/// The banner's `os.name`, `os.version`, `os.arch`, core count, ... (`TLC.java:1560-1596`).
pub struct HostInfo {
    pub os_name: String,
    pub os_version: String,
    pub os_arch: String,
    pub cores: usize,
}

impl Env {
    /// The real process: system clock, OS entropy, this host.
    #[expect(clippy::disallowed_methods, reason = "the one place tlars reads the real process")]
    pub fn process() -> Self {
        Env {
            clock: Clock::System,
            pid: std::process::id(),
            temp_dir: std::env::temp_dir(),
            entropy: Entropy::Os,
            worker_stack_bytes: 64 << 20,
            host: HostInfo::current(),
            install_location: std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
            locale: tlars_java::Locale::from_environment(|name| std::env::var(name).ok()),
        }
    }

    /// Fixed clock, pid and entropy: output is a function of the arguments and files.
    #[expect(clippy::disallowed_methods, reason = "SANY needs a real directory to copy the standard modules to")]
    pub fn hermetic() -> Self {
        Env {
            clock: Clock::Fixed { epoch_millis: 0, utc_offset_minutes: 0 },
            pid: 0,
            // A real directory: SANY writes the standard modules there. Their paths only
            // reach the `Parsing file` lines.
            temp_dir: std::env::temp_dir(),
            entropy: Entropy::Fixed(0),
            worker_stack_bytes: 64 << 20,
            host: HostInfo { os_name: "Linux".into(), os_version: "0".into(), os_arch: "amd64".into(), cores: 1 },
            install_location: String::new(),
            // The locale TLC's conformance recordings run under.
            locale: tlars_java::Locale::c_utf8(),
        }
    }

    /// `MP.now()`: `yyyy-MM-dd HH:mm:ss`, or `NOW` under `-Dtlc2.output.MP.noTimestamps=true`.
    fn now_formatter(&self, no_timestamps: bool) -> tlars_msg::Now {
        if no_timestamps {
            return Box::new(|| "NOW".to_string());
        }
        match self.clock {
            Clock::Fixed { epoch_millis, utc_offset_minutes } => {
                let text = tlars_java::time::format(tlars_java::time::Pattern::Timestamp, epoch_millis, utc_offset_minutes);
                Box::new(move || text.clone())
            }
            // TODO(M3): the system time zone (`ZoneId.systemDefault()`); UTC until then.
            Clock::System => Box::new(|| tlars_java::time::format(tlars_java::time::Pattern::Timestamp, Clock::System.now_millis(), 0)),
        }
    }
}

impl Entropy {
    /// Seed for the `java.util.Random` TLC draws its defaults from.
    fn seed(&self) -> i64 {
        match self {
            Entropy::Fixed(n) => *n as i64,
            Entropy::Os => {
                use std::hash::{BuildHasher, Hasher};
                std::collections::hash_map::RandomState::new().build_hasher().finish() as i64
            }
        }
    }
}

/// `Boolean.getBoolean(name)`: the property is set to `true`, ignoring case.
fn property_is_true(properties: &[(String, String)], name: &str) -> bool {
    properties.iter().rev().find(|(k, _)| k == name).is_some_and(|(_, v)| v.eq_ignore_ascii_case("true"))
}

impl HostInfo {
    /// Java's `os.name`, `os.version`, `os.arch` and `availableProcessors()` for this host.
    fn current() -> Self {
        let os_name = match std::env::consts::OS {
            "linux" => "Linux",
            "macos" => "Mac OS X",
            "windows" => "Windows",
            other => other,
        };
        let os_arch = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            other => other,
        };
        let os_version = std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default();
        HostInfo {
            os_name: os_name.into(),
            os_version,
            os_arch: os_arch.into(),
            cores: std::thread::available_parallelism().map_or(1, usize::from),
        }
    }
}

/// `TLCGlobals.Version.get()` plus the revision tlars reproduces, as `printWelcome` prints it.
pub(crate) const VERSION: &str = concat!("Version ", env!("CARGO_PKG_VERSION"), " of tlars (rev: bc1b506)");

/// Java system properties given as leading `-D<name>=<value>` arguments.
fn split_system_properties(args: &[String]) -> (Vec<(String, String)>, &[String]) {
    let count = args.iter().take_while(|a| a.starts_with("-D")).count();
    let props = args[..count]
        .iter()
        .map(|a| {
            let kv = &a[2..];
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.to_string(), v.to_string())
        })
        .collect();
    (props, &args[count..])
}

/// `TLC.main`: `handleParameters`, then `process()`, then `EC.ExitStatus`.
pub fn run(inv: Invocation<'_>) -> ExitStatus {
    let (mut properties, args) = split_system_properties(&inv.args);
    properties.extend(inv.system_properties.iter().cloned());
    let no_timestamps = property_is_true(&properties, "tlc2.output.MP.noTimestamps");
    match cli::parse(args, inv.env.host.cores) {
        Ok(options) => {
            let stack = inv.env.worker_stack_bytes;
            let session = session::Session::new(inv, options, properties);
            // Evaluation recurses once per nesting level; it gets a worker-sized stack so the
            // evaluator's own depth guard, not the native stack, decides when TLC's
            // StackOverflowError happens.
            let exit = std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .name("main".into())
                    .stack_size(stack)
                    .spawn_scoped(scope, || driver::process(&session))
                    .expect("spawning the TLC main thread")
                    .join()
                    .unwrap_or_else(|e| std::panic::resume_unwind(e))
            });
            match exit {
                driver::Exit::Returned(ec) => session.exit_status(ec),
                driver::Exit::System(status) => status,
            }
        }
        Err(cli::CliExit::Usage { message, tool_mode }) => {
            // `printErrorMsg`: the welcome line, then 1102.
            let reporter = tlars_msg::Reporter::new(
                inv.stdout,
                tlars_msg::ReporterConfig {
                    tool_mode,
                    warnings_enabled: true,
                    suppressed: Vec::new(),
                    as_errors: Vec::new(),
                    debug: false,
                    now: inv.env.now_formatter(no_timestamps),
                    unimplemented: None,
                },
            );
            use tlars_msg::Report;
            reporter.message(tlars_msg::Ec::TLC_VERSION, &[VERSION]);
            reporter.error(tlars_msg::Ec::WRONG_COMMANDLINE_PARAMS_TLC, &[&message]);
            ExitStatus::UsageError
        }
        Err(cli::CliExit::Help) => todo!("M5: TLC.printUsage"),
        Err(cli::CliExit::Crash { exception }) => {
            let mut stderr = inv.stderr;
            let _ = writeln!(stderr, "Exception in thread \"main\" {exception}");
            ExitStatus::UsageError
        }
    }
}
