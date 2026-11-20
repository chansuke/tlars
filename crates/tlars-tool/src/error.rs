//! Java's exception taxonomy as values. TLC uses exceptions for control flow
//! (`InvariantViolatedException` thrown out of `addElement`), so the generators return
//! [`Flow`] and sinks stop generation with [`Halt::Stop`].

use tlars_msg::Ec;
use tlars_sany::graph::Loc;

/// What kind of Java throwable TLC would have seen; decides which message and exit code
/// `ModelChecker.doNextFailed` / `Worker` choose.
#[derive(Clone, Debug)]
pub enum EvalErrorKind {
    /// `Assert.TLCRuntimeException` (most `Assert.fail(ec, ...)`).
    Runtime,
    /// `EvalException` (has its own error code, e.g. `TLC_VALUE_ASSERT_FAILED`).
    Eval,
    /// `FingerprintException`: a chain of value sources, printed as a nested-expression trace.
    Fingerprint { sources: Vec<Loc> },
    /// `StackOverflowError`: raised by `Ev`'s depth guard, never by the native stack.
    StackOverflow,
    /// `AssertionError` → `TLC_BUG`.
    Assertion,
    /// A construct tlars does not evaluate yet. It travels like TLC's exceptions, so code
    /// that swallows failures (constant folding) swallows it too; anywhere else it surfaces
    /// as an error whose text TLC never prints.
    Unimplemented,
}

/// A failed evaluation. `message` is already TLC's text (`tlars_msg::render` at the throw
/// site, as `Assert.fail` builds it).
#[derive(Clone, Debug)]
pub struct EvalError {
    pub ec: Ec,
    pub message: String,
    pub loc: Option<Loc>,
    pub kind: EvalErrorKind,
}

pub type EvalResult<T> = Result<T, EvalError>;

impl EvalError {
    /// `new EvalException(ec, params)`; `message` is `MP.getMessage(ec, params)`.
    pub(crate) fn eval(ec: Ec, message: String) -> EvalError {
        EvalError { ec, message, loc: None, kind: EvalErrorKind::Eval }
    }

    /// `Assert.fail(ec, params)`; `message` is `MP.getMessage(ec, params)`.
    pub(crate) fn runtime(ec: Ec, message: String) -> EvalError {
        EvalError { ec, message, loc: None, kind: EvalErrorKind::Runtime }
    }

    /// `Assert.fail(reason, expr, ctxt)`: a `TLCDetailedRuntimeException` with
    /// `EC.GENERAL` and the reason as its message.
    pub(crate) fn detailed(reason: String, loc: Option<Loc>) -> EvalError {
        EvalError { ec: Ec::GENERAL, message: reason, loc, kind: EvalErrorKind::Runtime }
    }

    pub(crate) fn unimplemented(what: String) -> EvalError {
        EvalError { ec: Ec::GENERAL, message: format!("tlars does not implement {what} yet"), loc: None, kind: EvalErrorKind::Unimplemented }
    }
}

/// Why state generation stopped early.
#[derive(Debug)]
pub enum Halt {
    /// A sink decided to stop (violation recorded, `-continue` off). Not an error here.
    Stop,
    /// Evaluation failed; the caller reports it.
    Eval(EvalError),
}

impl From<EvalError> for Halt {
    fn from(e: EvalError) -> Self {
        Halt::Eval(e)
    }
}

pub type Flow = Result<(), Halt>;
