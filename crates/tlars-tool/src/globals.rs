//! `TLCGlobals` + the `-D` system properties TLC reads, as one value built once by the CLI
//! and stored in the `Tool`. No statics: two sessions in one process never share knobs.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LnCheck {
    Default,
    Final,
    SeqFinal,
    Off,
}

#[derive(Clone, Debug)]
pub struct TlcGlobals {
    pub tool_mode: bool,
    pub workers: usize,
    pub check_deadlock: bool,
    pub continue_after_violation: bool,
    pub difftrace: bool,
    /// `-view` (`TLCGlobals.useView`): a trace state prints its VIEW instead of its
    /// variables.
    pub use_view: bool,
    /// `-terse` clears it: lazy sets are not expanded when printed.
    pub expand: bool,
    /// `TLCGlobals.enumBound` (2000).
    pub enum_bound: i32,
    /// `tlc2.value.Values.width` (80).
    pub width: usize,
    /// `TLCGlobals.warn`.
    pub warn: bool,
    pub max_set_size: i32,
    pub lncheck: LnCheck,
    /// `-Dtlc2.tool.impl.SpecProcessor.aggressiveConstantCaching`.
    pub aggressive_constant_caching: bool,
    pub fp_index: usize,
    pub seed: i64,
    pub aril: i64,
    pub no_timestamps: bool,
    /// The `-D` properties the CLI was given, in the order given (a later one wins). What
    /// `TLCGet("-Dkey")` falls back to when nothing set that key at run time.
    pub properties: Vec<(String, String)>,
    /// `TLCGlobals.Version.installLocation()`, reported by `TLCGet("config").install`.
    pub install_location: String,
    /// The directory the process was started in, which is what `open(2)` resolves a
    /// relative `new File(path)` against (`Json!JsonSerialize` and the other operators that
    /// name a file). Not `user.dir`: the JDK only consults that property to *print* an
    /// absolute path, never to open one.
    pub cwd: std::path::PathBuf,
}
