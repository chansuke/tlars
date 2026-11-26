//! The `Tool` the value goldens run against: the smallest module SANY accepts, an empty cfg,
//! and an intern table whose tokens are the ones the Java golden recorded.
//!
//! A value method reaches the tool for the interner, the FP64 polynomial, the model value
//! table and `vars[]`, so a golden that compares bytes or fingerprints needs one. Nothing
//! here is a fixture the compared code can see: the values are built by each golden's own
//! list.

use super::ModelValue;
use crate::eval::Ev;
use std::sync::Arc;

pub(super) struct Fixture {
    pub(super) tool: crate::tool::Tool,
    pub(super) model_values: Vec<ModelValue>,
}

impl Fixture {
    /// `model_values` and `strings` are `(token, text)` as the golden recorded them. A string
    /// is interned at the position the recording JVM gave it, with fillers in between, because
    /// a model value fingerprints as its token and `UniqueString.write` writes one.
    pub(super) fn new(poly: usize, model_values: &[(i32, String)], strings: &[(i32, String)]) -> Fixture {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
        let interner = Arc::new(tlars_sany::Interner::new());
        let mut wanted: Vec<&(i32, String)> = model_values.iter().chain(strings).collect();
        wanted.sort_by_key(|(tok, _)| *tok);
        wanted.dedup_by_key(|(tok, _)| *tok);
        for (tok, text) in wanted {
            while interner.len() as i32 + 1 < *tok {
                interner.intern(&format!("\u{0}filler{}", interner.len()));
            }
            assert_eq!(interner.intern(text).tok(), *tok, "{text}");
        }
        let graph = tlars_sany::load(
            tlars_sany::LoadRequest {
                root_file: "FpGolden.tla",
                user_dir: Some(std::path::Path::new(dir)),
                working_dir: std::path::Path::new(dir),
                library_paths: &[],
                interner: interner.clone(),
                extra_root_extends: &[],
                suppressed_codes: &[],
                messages_as_errors: &[],
            },
            &mut Silent,
        )
        .expect("the fixture module loads");
        let out = report();
        let config = tlars_sany::cfg::parse(b"", &interner).expect("an empty config parses");
        let tool = crate::spec::build_tool(crate::spec::ToolInput {
            graph,
            config,
            globals: globals(poly),
            extras: Default::default(),
            stdin: Box::new(std::io::empty()),
            out: &out,
        })
        .expect("the fixture module builds a tool");
        // `ModelValue.make` in creation order, which is the index the value stream writes and
        // the slot `ModelValue.mvs[i]` reads back.
        let model_values =
            model_values.iter().map(|(_, name)| tool.add_model_value(interner.intern(name), name).expect("a model value")).collect();
        Fixture { tool, model_values }
    }

    /// An evaluator over the fixture, for one golden case.
    pub(super) fn with_ev<T>(&self, f: impl FnOnce(&mut Ev<'_>) -> T) -> T {
        let out = report();
        let mut locals = crate::eval::WorkerLocals::new(0, 4);
        f(&mut self.tool.ev(&mut locals, &out))
    }
}

fn globals(poly: usize) -> crate::globals::TlcGlobals {
    crate::globals::TlcGlobals {
        tool_mode: false,
        workers: 1,
        check_deadlock: true,
        continue_after_violation: false,
        difftrace: false,
        use_view: false,
        expand: true,
        enum_bound: 2000,
        width: 80,
        warn: true,
        max_set_size: 1_000_000,
        lncheck: crate::globals::LnCheck::Final,
        aggressive_constant_caching: false,
        fp_index: poly,
        seed: 4,
        aril: 0,
        no_timestamps: true,
        properties: Vec::new(),
        install_location: String::new(),
        cwd: std::path::PathBuf::new(),
        locale: tlars_java::Locale::c_utf8(),
    }
}

pub(super) fn report() -> tlars_msg::Reporter<'static> {
    tlars_msg::Reporter::new(
        Box::new(std::io::sink()),
        tlars_msg::ReporterConfig {
            tool_mode: false,
            warnings_enabled: false,
            suppressed: Vec::new(),
            as_errors: Vec::new(),
            debug: false,
            now: Box::new(|| "NOW".into()),
            unimplemented: None,
        },
    )
}

struct Silent;

impl tlars_sany::SanyOut for Silent {
    fn log_line(&mut self, _text: &str) {}
    fn line(&mut self, _text: &str) {}
    fn err_line(&mut self, _text: &str) {}
}
