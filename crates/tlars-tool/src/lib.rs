//! TLC's `ITool` side, mirrored: SpecProcessor binding, the tree-walking evaluator
//! (`Tool.java`), values (`tlc2.value.impl`), FP64, the pretty printer, native modules.
//!
//! Public surface = what `ModelChecker`/`Worker`/`LiveCheck`/`Simulator` call on `ITool`:
//! [`build_tool`] to obtain a frozen [`Tool`], [`Tool::ev`] to get a per-worker [`Ev`], and
//! the `Ev` methods (state generation, checks, fingerprint, replay, printing). Everything
//! about *how* evaluation works stays private.
#![allow(dead_code, unused_variables)]

mod actions;
mod ctx;
mod effects;
mod error;
mod eval;
mod fp64;
mod fp64_polys;
mod globals;
mod live;
mod modules;
mod spec;
mod state;
mod symmetry;
mod tool;
mod value;

pub use actions::{Action, ActionId};
pub use effects::MainChecker;
pub use error::{EvalError, EvalErrorKind, EvalResult, Flow, Halt};
pub use eval::{Aliased, CallStack, EvalControl, Ev, InitSink, NextSink, StateInfo, WorkerLocals};
pub use fp64::{Fp, FpKey};
pub use globals::{LnCheck, TlcGlobals};
pub use live::{CheckBits, LiveSpec, Pem, Solution, Tableau};
pub use spec::{CliSpecExtras, PostCondition, RuntimeInvariant, SpecError, ToolInput, TraceConstraint, build_tool};
pub use state::{PartialState, State, StateRef, VarSlot};
pub use tool::{CheckId, ImpliedAction, ImpliedInit, Invariant, Temporal, Tool};
pub use value::Value;

// Values, contexts and states cross worker threads through the shared queue; the tool is
// shared by all workers. These must hold for the engine's design to be sound.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Value>();
    assert_send_sync::<crate::ctx::Ctx>();
    assert_send_sync::<State>();
    assert_send_sync::<Tool>();
};

// Value stays two words; Option<Value> uses the tag niche.
const _: () = assert!(std::mem::size_of::<Value>() == 16);
const _: () = assert!(std::mem::size_of::<Option<Value>>() == 16);
