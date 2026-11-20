//! Per-run replacement for every Java static TLC depends on.

use crate::cli::Options;
use crate::{Env, Invocation, property_is_true};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tlars_msg::{Ec, ExitStatus, Reporter, ReporterConfig};
use tlars_sany::Interner;

pub(crate) struct Session<'a> {
    pub(crate) reporter: Reporter<'a>,
    /// `ToolIO.err`.
    pub(crate) stderr: Mutex<Box<dyn Write + Send + 'a>>,
    /// Created before the cfg is parsed: cfg strings take the first tokens.
    pub(crate) interner: Arc<Interner>,
    pub(crate) options: Options,
    pub(crate) env: Env,
    pub(crate) cwd: PathBuf,
    /// Java system properties, in the order given (a later one wins).
    pub(crate) properties: Vec<(String, String)>,
    /// `TLC.startTime`, taken at the end of `handleParameters` (`TLC.java:1194`).
    pub(crate) start_millis: i64,
    /// `-fp`, or the TLC constructor's `new Random().nextInt(FP64.Polys.length)`.
    pub(crate) fp_index: usize,
    /// `-seed`, or `process()`'s `rng.nextLong()`.
    pub(crate) seed: i64,
}

impl<'a> Session<'a> {
    pub(crate) fn new(inv: Invocation<'a>, options: Options, system_properties: Vec<(String, String)>) -> Self {
        let no_timestamps = property_is_true(&system_properties, "tlc2.output.MP.noTimestamps");
        let reporter = Reporter::new(
            inv.stdout,
            ReporterConfig {
                tool_mode: options.tool_mode,
                warnings_enabled: options.warnings_enabled,
                suppressed: options.suppressed.clone(),
                as_errors: options.messages_as_errors.clone(),
                debug: property_is_true(&system_properties, "tlc2.output.MP.noDebug"),
                now: inv.env.now_formatter(no_timestamps),
                unimplemented: inv.unimplemented.clone(),
            },
        );
        let mut rng = tlars_java::JavaRandom::new(inv.env.entropy.seed());
        let fp_index = options.fp_index.unwrap_or_else(|| rng.next_int_bounded(crate::cli::MAX_FP_INDEX as i32 + 1) as usize);
        let seed = options.seed.unwrap_or_else(|| rng.next_long());
        // TODO(M3): `FileUtil.makeMetaDir` creates the states directory here; nothing writes
        // to it until the FPSet and trace files exist.
        Session {
            reporter,
            stderr: Mutex::new(inv.stderr),
            interner: Arc::new(Interner::new()),
            start_millis: inv.env.clock.now_millis(),
            options,
            env: inv.env,
            cwd: inv.cwd,
            properties: system_properties,
            fp_index,
            seed,
        }
    }

    pub(crate) fn property(&self, name: &str) -> Option<&str> {
        self.properties.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub(crate) fn property_is_true(&self, name: &str) -> bool {
        property_is_true(&self.properties, name)
    }

    pub(crate) fn now_millis(&self) -> i64 {
        self.env.clock.now_millis()
    }

    /// `EC.ExitStatus.errorConstantToExitStatus` (`TLC.java:384`). Under `-continue` the
    /// checker itself returns `NO_ERROR` for violations; every other error keeps its status.
    pub(crate) fn exit_status(&self, ec: Ec) -> ExitStatus {
        ec.exit_status()
    }
}
