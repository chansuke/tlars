//! `tlc2.value.impl`, mirrored: one [`Value`] variant per TLC value class.
//!
//! Why mirror the classes: the representation is observable. `1..3` prints as `1..3` and
//! `{1,2,3}` as `{1, 2, 3}`; `x' \in S` enumerates a lazy `SetCup` with duplicates; the
//! error text of `a = b` depends on the *receiver's* class.
//!
//! Invariants encoded in types:
//! - `Value` implements **no** `PartialEq`/`Eq`/`Ord`/`Hash`. TLC's equality and order are
//!   fallible (incomparable kinds are errors) and need the evaluator to force closures, so
//!   they are `tlc_equals`/`tlc_compare` (see `order`). Nobody can write `==` or key a std
//!   map by a value and get silently wrong semantics.
//! - Evaluation-only classes (LazyValue, OpLambda, Method, OpRcd) are `ctx::Binding`s.
//! - Payloads are immutable; the only interior mutability is write-once `OnceLock`
//!   caches (normalised order, converted lazy sets, function index), whose contents are a
//!   pure function of the payload. TLC's in-place `normalize()` becomes "once normalised,
//!   only the normalised form is read".
//! - No hash-consing: it would merge representations whose printed forms differ.

mod enumerate;
mod fingerprint;
mod order;
mod print;


use crate::ctx::Ctx;
use crate::eval::{EvalControl, Ev};
use crate::error::EvalResult;
use crate::state::Snapshot;
use std::sync::{Arc, OnceLock};
use tlars_sany::Atom;
use tlars_sany::graph::{ExprId, ParamId, SymbolRef};

/// `ValueConstants` kind bytes. The discriminants go into fingerprints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum ValueKind {
    Bool = 0,
    Int = 1,
    Real = 2,
    String = 3,
    Record = 4,
    SetEnum = 5,
    SetPred = 6,
    Tuple = 7,
    FcnLambda = 8,
    FcnRcd = 9,
    OpLambda = 10,
    OpRcd = 11,
    Method = 12,
    SetOfFcns = 13,
    SetOfRcds = 14,
    SetOfTuples = 15,
    Subset = 16,
    SetDiff = 17,
    SetCap = 18,
    SetCup = 19,
    Union = 20,
    Model = 21,
    User = 22,
    Interval = 23,
    Undef = 24,
    Lazy = 25,
    Dummy = 26,
}

#[derive(Clone)]
pub enum Value {
    Bool(bool),
    /// 32-bit, overflow is an error (`TLC_MODULE_OVERFLOW`).
    Int(i32),
    /// `StringValue`: compares by intern token.
    Str(Atom),
    Model(ModelValue),
    Tuple(Arc<TupleValue>),
    Record(Arc<RecordValue>),
    FcnRcd(Arc<FcnRcdValue>),
    SetEnum(Arc<SetEnumValue>),
    Interval(IntervalValue),
    Subset(Arc<SubsetValue>),
    KSubset(Arc<KSubsetValue>),
    SetOfFcns(Arc<SetOfFcnsValue>),
    SetOfRcds(Arc<SetOfRcdsValue>),
    SetOfTuples(Arc<SetOfTuplesValue>),
    Union(Arc<UnionValue>),
    SetCup(Arc<SetBinValue>),
    SetCap(Arc<SetBinValue>),
    SetDiff(Arc<SetBinValue>),
    SetPred(Arc<SetPredValue>),
    FcnLambda(Arc<FcnLambdaValue>),
    User(Arc<UserValue>),
    Undef,
}

/// `ModelValue`: name token, creation index (`ModelValue.index`, used by symmetry
/// permutations), and the optional type char of typed model values (`a_x`).
#[derive(Clone, Copy, Debug)]
pub struct ModelValue {
    pub(crate) name: Atom,
    pub(crate) index: u16,
    pub(crate) ty: Option<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct IntervalValue {
    pub(crate) lo: i32,
    pub(crate) hi: i32,
}

pub struct TupleValue {
    pub(crate) elems: Box<[Value]>,
}

/// `SetEnumValue`: elements as constructed (possibly unsorted, with duplicates) plus the
/// normalised (sorted, deduplicated) form once computed. A failed normalisation (e.g.
/// `{1, "a"}`) is not cached, so the error re-surfaces at the next use, as in TLC.
pub struct SetEnumValue {
    pub(crate) built: Box<[Value]>,
    pub(crate) norm: OnceLock<Box<[Value]>>,
}

/// Fields as constructed; normalised = sorted by name token.
pub struct RecordValue {
    pub(crate) names: Box<[Atom]>,
    pub(crate) values: Box<[Value]>,
    pub(crate) norm: OnceLock<(Box<[Atom]>, Box<[Value]>)>,
}

pub enum FcnDomain {
    Explicit(Box<[Value]>),
    /// `FcnRcdValue.intv`: printed as a tuple when `lo == 1`.
    Interval { lo: i32, hi: i32 },
}

pub struct FcnRcdValue {
    pub(crate) domain: FcnDomain,
    pub(crate) values: Box<[Value]>,
    pub(crate) norm: OnceLock<(Box<[Value]>, Box<[Value]>)>,
}

/// Lazy sets. `converted` is `convertAndCache()`'s SetEnum.
pub struct SubsetValue {
    pub(crate) base: Value,
    pub(crate) converted: OnceLock<Arc<SetEnumValue>>,
}

pub struct KSubsetValue {
    pub(crate) base: Value,
    pub(crate) k: i32,
    pub(crate) converted: OnceLock<Arc<SetEnumValue>>,
}

pub struct SetOfFcnsValue {
    pub(crate) domain: Value,
    pub(crate) range: Value,
    pub(crate) converted: OnceLock<Arc<SetEnumValue>>,
}

/// Field names sorted by token at construction (`SetOfRcdsValue(..., isNorm=false)`).
pub struct SetOfRcdsValue {
    pub(crate) names: Box<[Atom]>,
    pub(crate) sets: Box<[Value]>,
    pub(crate) converted: OnceLock<Arc<SetEnumValue>>,
}

pub struct SetOfTuplesValue {
    pub(crate) sets: Box<[Value]>,
    pub(crate) converted: OnceLock<Arc<SetEnumValue>>,
}

pub struct UnionValue {
    pub(crate) set: Value,
    pub(crate) converted: OnceLock<Arc<SetEnumValue>>,
}

/// SetCup / SetCap / SetDiff, built only when neither side is Reducible.
pub struct SetBinValue {
    pub(crate) left: Value,
    pub(crate) right: Value,
    pub(crate) converted: OnceLock<Arc<SetEnumValue>>,
}

/// A captured evaluation environment: TLC's `(tool, con, state, pstate, control)` minus the
/// tool, which is passed explicitly as `Ev` when the closure is forced.
pub struct Closure {
    pub(crate) ctx: Ctx,
    pub(crate) s0: Option<Arc<Snapshot>>,
    pub(crate) s1: Option<Arc<Snapshot>>,
    pub(crate) control: EvalControl,
}

/// `{x \in S : P}` over a non-Reducible `S`.
pub struct SetPredValue {
    pub(crate) vars: Box<[ParamId]>,
    pub(crate) tuple: bool,
    pub(crate) inner: Value,
    pub(crate) pred: ExprId,
    pub(crate) env: Closure,
    pub(crate) converted: OnceLock<Arc<SetEnumValue>>,
}

/// `[x \in S |-> e]` kept lazy (non-Reducible domain, or KeepLazy), incl. recursive
/// functions. Recursion: TLC stores the function in its own context (`makeRecursive`), a
/// cycle that would leak under `Arc`. Here `rec_name` is kept and `f := self` is consed onto
/// the context *at apply time*: same semantics, no cycle.
pub struct FcnLambdaValue {
    pub(crate) params: Box<[(Box<[ParamId]>, bool, Value)]>,
    pub(crate) body: ExprId,
    pub(crate) env: Closure,
    pub(crate) rec_name: Option<SymbolRef>,
    pub(crate) excepts: Box<[(Box<[Value]>, Value)]>,
    pub(crate) converted: OnceLock<Arc<FcnRcdValue>>,
}

/// `UserValue(UserObj)`: the infinite / special sets TLC's Java modules provide.
pub enum UserValue {
    Nat,
    Int,
    Real,
    Seq(Value),
    String,
    AnySet,
}

impl Value {
    pub(crate) fn kind(&self) -> ValueKind {
        todo!()
    }

    pub(crate) fn tuple(elems: Vec<Value>) -> Value {
        Value::Tuple(Arc::new(TupleValue { elems: elems.into_boxed_slice() }))
    }

    /// Unnormalised SetEnum, as `{e : x \in S}` and cfg `{...}` produce.
    pub(crate) fn set_enum(elems: Vec<Value>) -> Value {
        Value::SetEnum(Arc::new(SetEnumValue { built: elems.into_boxed_slice(), norm: OnceLock::new() }))
    }

    /// `toTuple()` with the calling operator's error text on failure, e.g.
    /// "The first argument of Append must be a sequence, but instead it is: ...".
    pub(crate) fn to_tuple_elems(&self, ev: &mut Ev<'_>, op: &str, arg_pos: u32) -> EvalResult<Vec<Value>> {
        todo!()
    }

    /// `toSetEnum()`: `None` if this value is not an enumerable set.
    pub(crate) fn to_set_enum(&self, ev: &mut Ev<'_>) -> EvalResult<Option<Arc<SetEnumValue>>> {
        todo!()
    }

    /// `toFcnRcd()`: tuples, records and enumerable FcnLambdas as functions.
    pub(crate) fn to_fcn_rcd(&self, ev: &mut Ev<'_>) -> EvalResult<Option<Arc<FcnRcdValue>>> {
        todo!()
    }

    /// `Reducible`: only Interval and SetEnum; decides eager vs lazy set operators.
    pub(crate) fn is_reducible(&self) -> bool {
        matches!(self, Value::Interval(_) | Value::SetEnum(_))
    }

    /// `deepNormalize()`: done before a state is fingerprinted / shared.
    pub(crate) fn deep_normalize(&self, ev: &mut Ev<'_>) -> EvalResult<()> {
        match self {
            Value::Bool(_) | Value::Int(_) | Value::Str(_) | Value::Model(_) | Value::Interval(_) => Ok(()),
            _ => Err(crate::error::EvalError::unimplemented("deepNormalize of compound values (M2 unit 3)".into())),
        }
    }

    /// `permute(MVPerm)`.
    pub(crate) fn permute(&self, ev: &mut Ev<'_>, perm: &crate::symmetry::MvPerm) -> EvalResult<Value> {
        todo!()
    }
}
