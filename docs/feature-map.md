# Feature map

The names to use for features in commits and pull requests, the milestone that delivers
each (`docs/DESIGN.md`, milestone table), the TLC classes it reproduces, and the module that
owns it. Who owns each ordering or encoding invariant is in the DESIGN owner table; this
page does not repeat it.

| Feature | Milestone | TLC classes | Owner |
|---|---|---|---|
| Java utilities (Hashtable order, `String.hashCode`, Random, number and date formats, pprint, gzip) | M0 | `java.util.*`, `tlc2.util.*` | `tlars-java` |
| Messages and exit codes | M0 | `tlc2.output.EC`, `MP` | `tlars-msg` (generated `catalog.rs`) |
| Command line | M0, M5 | `TLC.handleParameters` | `tlars-check::cli` |
| Host and determinism (`Env`, `Clock`) | M0 | `System`, JVM properties | `tlars-check::Env`, `tlars_java::time::Clock`, `tlars_sany::Host` |
| Fingerprints | M0, M2 | `tlc2.util.FP64` | `tlars-tool::fp64`, `value/fingerprint.rs` |
| Parser and lexer | M1 | `TLAplusParser`, `TokenManager` | generated `tlars-sany/src/parser/generated.rs` (`xtask jj2rs`) |
| Module loading | M1 | `SimpleFilenameToStream`, `ToolIO` | `tlars-sany::parser::files` |
| Semantic analysis, level checking, linting | M1 | `tla2sany.semantic.*` | translated in `generated.rs`, frozen by `tlars-sany::parser::freeze` into `graph` |
| PlusCal translation check | M1 | `pcal.Validator` | `tlars-sany::parser::pcal_validator` |
| Config file | M1 | `ModelConfig` | `tlars-sany::cfg` |
| Values | M2 | `tlc2.value.impl.*` | `tlars-tool::value` |
| Standard modules in Rust | M2 | `tlc2.module.*` | `tlars-tool::modules` |
| Constant evaluation | M2 | `Tool.eval*` | `tlars-tool::eval::expr` |
| Spec processing | M3 | `SpecProcessor`, `Tool` constructor | `tlars-tool::spec` |
| Action splitting | M3 | `Action`, `Tool.getActions` | `tlars-tool::actions` |
| Init and Next | M3 | `Tool.getInitStates`, `getNextStates` | `tlars-tool::eval::{init, next}` |
| Effects (RNG, TLCSet/TLCGet, Print) | M3 | `IdThread`, `TLCExt` | `tlars-tool::effects` |
| Symmetry | M3 | `MVPerm` | `tlars-tool::symmetry` |
| BFS checker | M3 | `ModelChecker`, `Worker`, `AbstractChecker` | `tlars-check::bfs` |
| State queue, fingerprint set, trace | M3 | `StateQueue`, `FPSet`, `ConcurrentTLCTrace` | `tlars-check::{queue, fpset, trace}` |
| Per-run state | M3 | every static TLC relies on | `tlars-check::session` |
| Performance gate | M3.5 | | `xtask bench`, `bench/` |
| Liveness formulas | M4 | `Liveness`, `TBGraph` | `tlars-tool::live` |
| Liveness graph and lassos | M4 | `LiveCheck`, `LiveWorker` | `tlars-check::liveness`, `tlars-check::print` |
| Simulation | M5 | `Simulator` | `tlars-check::simulate` |
| Depth-first iterative deepening | M5 | `DFIDModelChecker` | `tlars-check::dfid` |
| In-order parallel commit, disk FPSet, checkpoints | M6 | `FPSet` variants, `DiskStateQueue` | `tlars-check` (`SeenOracle`) |
| Conformance corpus | all | | `tlars-conformance`, `xtask conformance` |
