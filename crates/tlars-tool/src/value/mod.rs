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

pub(crate) mod enumerate;
mod fingerprint;
mod function;
mod order;
mod print;

pub(crate) use function::{Except, warn};

use crate::ctx::Ctx;
use crate::error::{EvalError, EvalResult};
use crate::eval::{EvalControl, Ev};
use crate::state::{Snapshot, StateRef};
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

impl IntervalValue {
    /// `IntervalValue.size()` (`IntervalValue.java:120-137`): counted in `long`, so an
    /// interval of more than `Integer.MAX_VALUE` elements fails instead of wrapping.
    pub(crate) fn size(&self, ev: &mut Ev<'_>) -> EvalResult<i32> {
        if self.hi < self.lo {
            return Ok(0);
        }
        let sz = i64::from(self.hi) - i64::from(self.lo) + 1;
        if sz > i64::from(i32::MAX) {
            let shown = crate::value::print::ppr(ev, &format!("{}..{}", self.lo, self.hi))?;
            return Err(EvalError::detailed(
                format!("Size of interval value exceeds the maximum representable size (32bits): {shown}."),
                None,
            ));
        }
        Ok(sz as i32)
    }

    /// `high < low`, which every caller of `size()` that only asks about emptiness uses.
    pub(crate) fn is_empty(&self) -> bool {
        self.hi < self.lo
    }

    /// `toSetEnum()`: the elements in order, already normalised.
    pub(crate) fn to_set_enum_arc(&self) -> Arc<SetEnumValue> {
        let elems: Box<[Value]> = (i64::from(self.lo)..=i64::from(self.hi)).map(|n| Value::Int(n as i32)).collect();
        let norm = OnceLock::new();
        let _ = norm.set(elems.clone());
        Arc::new(SetEnumValue { built: elems, norm })
    }
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
    /// `FcnRcdValue.intv`: printed as a tuple when `lo == 1`, and always normalised.
    Interval(IntervalValue),
}

/// Domain and values as constructed, plus the normalised (sorted by domain) pair once
/// computed. An interval domain is normalised by construction, so `norm` stays empty and
/// `is_normalized` reports the domain's kind. A failed sort (a duplicate domain element) is
/// not cached, so the error re-surfaces at the next use, as in TLC.
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

impl Closure {
    /// `new FcnLambdaValue(..., c, s0, s1, control)`, whose constructor copies both states.
    /// Copying them is M3, so a closure built while a state is bound is refused here instead
    /// of forcing with the wrong variables later.
    pub(crate) fn capture(ctx: &Ctx, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> EvalResult<Closure> {
        if !matches!((s0, s1), (StateRef::Empty, StateRef::Empty)) {
            return Err(EvalError::unimplemented("a function or set closure over a state (M3)".into()));
        }
        Ok(Closure { ctx: ctx.clone(), s0: None, s1: None, control })
    }

    /// The captured `(state, pstate)` as `eval` takes them.
    pub(crate) fn states(&self) -> (StateRef<'_>, StateRef<'_>) {
        debug_assert!(self.s0.is_none() && self.s1.is_none(), "capture refuses a closure over a state");
        (StateRef::Empty, StateRef::Empty)
    }
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
    /// `FcnParams`: one entry per `x, y \in S` list, as `(formals, isTuple, domain)`.
    pub(crate) params: Box<[(Box<[ParamId]>, bool, Value)]>,
    pub(crate) body: ExprId,
    pub(crate) env: Closure,
    pub(crate) rec_name: Option<SymbolRef>,
    pub(crate) excepts: Box<[Except]>,
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
        match self {
            Value::Bool(_) => ValueKind::Bool,
            Value::Int(_) => ValueKind::Int,
            Value::Str(_) => ValueKind::String,
            Value::Model(_) => ValueKind::Model,
            Value::Tuple(_) => ValueKind::Tuple,
            Value::Record(_) => ValueKind::Record,
            Value::FcnRcd(_) => ValueKind::FcnRcd,
            Value::SetEnum(_) => ValueKind::SetEnum,
            Value::Interval(_) => ValueKind::Interval,
            Value::Subset(_) | Value::KSubset(_) => ValueKind::Subset,
            Value::SetOfFcns(_) => ValueKind::SetOfFcns,
            Value::SetOfRcds(_) => ValueKind::SetOfRcds,
            Value::SetOfTuples(_) => ValueKind::SetOfTuples,
            Value::Union(_) => ValueKind::Union,
            Value::SetCup(_) => ValueKind::SetCup,
            Value::SetCap(_) => ValueKind::SetCap,
            Value::SetDiff(_) => ValueKind::SetDiff,
            Value::SetPred(_) => ValueKind::SetPred,
            Value::FcnLambda(_) => ValueKind::FcnLambda,
            Value::User(_) => ValueKind::User,
            Value::Undef => ValueKind::Undef,
        }
    }

    /// `Value.getKindString()` (`Value.java:34-62`).
    pub(crate) fn kind_string(&self) -> &'static str {
        match self.kind() {
            ValueKind::Bool => "a Boolean value",
            ValueKind::Int => "an integer",
            ValueKind::Real => "a real",
            ValueKind::String => "a string",
            ValueKind::Record => "a record",
            ValueKind::SetEnum => "a set of the form {e1, ... ,eN}",
            ValueKind::SetPred => "a set of the form {x \\in S : expr}",
            ValueKind::Tuple => "a tuple",
            ValueKind::FcnLambda => "a function of the form  [x \\in S |-> expr]",
            ValueKind::FcnRcd => "a function  of the form (d1 :> e1 @@ ... @@ dN :> eN)",
            ValueKind::OpLambda => "an operator",
            ValueKind::OpRcd => "a constant operator",
            ValueKind::Method => "a java method",
            ValueKind::SetOfFcns => "a set of the form [S -> T]",
            ValueKind::SetOfRcds => "a set of the form [d1 : S1, ... , dN : SN]",
            ValueKind::SetOfTuples => "a cartesian product",
            ValueKind::Subset => "a set of the form SUBSET S",
            ValueKind::SetDiff => "a set of the form S \\ T",
            ValueKind::SetCap => "a set of the form S \\cap T",
            ValueKind::SetCup => "a set of the form S \\cup T",
            ValueKind::Union => "a set of the form UNION  S",
            ValueKind::Model => "a model value",
            ValueKind::User => "a special set constant",
            ValueKind::Interval => "a set of the form i..j",
            ValueKind::Undef => "an undefined value",
            ValueKind::Lazy => "a value represented in lazy form",
            ValueKind::Dummy => "a dummy for not-a-value",
        }
    }

    /// The simple name of the `tlc2.value.impl` class, as cast errors print it.
    pub(crate) fn java_class(&self) -> &'static str {
        match self {
            Value::Bool(_) => "BoolValue",
            Value::Int(_) => "IntValue",
            Value::Str(_) => "StringValue",
            Value::Model(_) => "ModelValue",
            Value::Tuple(_) => "TupleValue",
            Value::Record(_) => "RecordValue",
            Value::FcnRcd(_) => "FcnRcdValue",
            Value::SetEnum(_) => "SetEnumValue",
            Value::Interval(_) => "IntervalValue",
            Value::Subset(_) => "SubsetValue",
            Value::KSubset(_) => "KSubsetValue",
            Value::SetOfFcns(_) => "SetOfFcnsValue",
            Value::SetOfRcds(_) => "SetOfRcdsValue",
            Value::SetOfTuples(_) => "SetOfTuplesValue",
            Value::Union(_) => "UnionValue",
            Value::SetCup(_) => "SetCupValue",
            Value::SetCap(_) => "SetCapValue",
            Value::SetDiff(_) => "SetDiffValue",
            Value::SetPred(_) => "SetPredValue",
            Value::FcnLambda(_) => "FcnLambdaValue",
            Value::User(_) => "UserValue",
            Value::Undef => "UndefValue",
        }
    }

    pub(crate) fn tuple(elems: Vec<Value>) -> Value {
        Value::Tuple(Arc::new(TupleValue { elems: elems.into_boxed_slice() }))
    }

    /// `new RecordValue(names, values, isNorm)`.
    pub(crate) fn record_arc(names: Vec<Atom>, values: Vec<Value>, is_norm: bool) -> Arc<RecordValue> {
        let (names, values) = (names.into_boxed_slice(), values.into_boxed_slice());
        let norm = OnceLock::new();
        if is_norm {
            let _ = norm.set((names.clone(), values.clone()));
        }
        Arc::new(RecordValue { names, values, norm })
    }

    /// `new FcnRcdValue(domain, values, isNorm)`.
    pub(crate) fn fcn_rcd_arc(domain: Vec<Value>, values: Vec<Value>, is_norm: bool) -> Arc<FcnRcdValue> {
        let (domain, values) = (domain.into_boxed_slice(), values.into_boxed_slice());
        let norm = OnceLock::new();
        if is_norm {
            let _ = norm.set((domain.clone(), values.clone()));
        }
        Arc::new(FcnRcdValue { domain: FcnDomain::Explicit(domain), values, norm })
    }

    /// `new FcnRcdValue(intv, values)`.
    pub(crate) fn fcn_rcd_interval_arc(intv: IntervalValue, values: Vec<Value>) -> Arc<FcnRcdValue> {
        Arc::new(FcnRcdValue { domain: FcnDomain::Interval(intv), values: values.into_boxed_slice(), norm: OnceLock::new() })
    }

    /// A SetEnum built normalised (`new SetEnumValue(elems, true)`).
    pub(crate) fn set_enum_normalized(elems: Vec<Value>) -> Value {
        Value::set_enum_of(elems, true)
    }

    /// `new SetEnumValue(vals, isNorm)`.
    pub(crate) fn set_enum_of(elems: Vec<Value>, is_norm: bool) -> Value {
        Value::SetEnum(set_enum_arc(elems, is_norm))
    }

    /// Unnormalised SetEnum, as `{e : x \in S}` and cfg `{...}` produce.
    pub(crate) fn set_enum(elems: Vec<Value>) -> Value {
        Value::set_enum_of(elems, false)
    }

    /// `toSetEnum()`: `None` if this value is not an enumerable set. A lazy set answers its
    /// cache when it has one, and otherwise enumerates itself *without* caching, as TLC's
    /// `toSetEnum` does; only `convertAndCache` publishes.
    pub(crate) fn to_set_enum(&self, ev: &mut Ev<'_>) -> EvalResult<Option<Arc<SetEnumValue>>> {
        let cache = match self {
            Value::SetEnum(s) => return Ok(Some(s.clone())),
            Value::Interval(i) => return Ok(Some(i.to_set_enum_arc())),
            Value::Subset(s) => s.converted.get(),
            Value::KSubset(s) => s.converted.get(),
            Value::SetOfFcns(s) => s.converted.get(),
            Value::SetOfRcds(s) => s.converted.get(),
            Value::SetOfTuples(s) => s.converted.get(),
            Value::Union(s) => s.converted.get(),
            Value::SetCup(s) | Value::SetCap(s) | Value::SetDiff(s) => s.converted.get(),
            Value::SetPred(s) => s.converted.get(),
            _ => return Ok(None),
        };
        if let Some(set) = cache {
            return Ok(Some(set.clone()));
        }
        // The `isNorm` each class passes to the SetEnum it builds: its own order is
        // normalised exactly when its operands' are.
        let is_norm = match self {
            Value::Subset(_) | Value::KSubset(_) => true,
            Value::SetCup(_) | Value::Union(_) => false,
            Value::SetCap(s) => s.left.is_normalized() && s.right.is_normalized(),
            Value::SetDiff(s) => s.left.is_normalized(),
            Value::SetOfFcns(s) => s.domain.is_normalized() && s.range.is_normalized(),
            Value::SetOfRcds(s) => s.sets.iter().all(Value::is_normalized),
            Value::SetOfTuples(s) => s.sets.iter().all(Value::is_normalized),
            Value::SetPred(s) => s.inner.is_normalized(),
            _ => unreachable!("only the lazy set classes reach this"),
        };
        // `SubsetValue.toSetEnum` sizes its array up front, which is how a power set may
        // exceed `setBound`; the others start at the default capacity and so cannot.
        let mut vals = match self {
            Value::Subset(_) => {
                let capacity = self.size(ev)?.max(0) as usize;
                ValueVec::with_capacity(ev, capacity)
            }
            _ => ValueVec::new(ev),
        };
        let mut elems = self.elements(ev, enumerate::EnumOrder::Native)?.expect("an enumerable value enumerates");
        while let Some(v) = elems.next(ev)? {
            vals.push(v)?;
        }
        Ok(Some(set_enum_arc(vals.into_vec(), is_norm)))
    }

    /// `convertAndCache()`: the enumerated, deep-normalised set, published once. Comparison,
    /// equality, cardinality and fingerprinting of a lazy set all go through it, which is
    /// why they leave the set in normalised order.
    pub(crate) fn convert_and_cache(&self, ev: &mut Ev<'_>) -> EvalResult<Arc<SetEnumValue>> {
        let cache = match self {
            Value::Subset(s) => &s.converted,
            Value::KSubset(s) => &s.converted,
            Value::SetOfFcns(s) => &s.converted,
            Value::SetOfRcds(s) => &s.converted,
            Value::SetOfTuples(s) => &s.converted,
            Value::Union(s) => &s.converted,
            Value::SetCup(s) | Value::SetCap(s) | Value::SetDiff(s) => &s.converted,
            Value::SetPred(s) => &s.converted,
            _ => unreachable!("convert_and_cache is for the lazy set classes"),
        };
        if let Some(set) = cache.get() {
            return Ok(set.clone());
        }
        let set = self.to_set_enum(ev)?.expect("a lazy set converts");
        // `SetPredValue.convertAndCache` normalises without descending; the others
        // deep-normalise.
        if matches!(self, Value::SetPred(_)) {
            order::normalized(ev, &set)?;
        } else {
            Value::SetEnum(set.clone()).deep_normalize(ev)?;
        }
        let _ = cache.set(set.clone());
        Ok(cache.get().expect("just published").clone())
    }

    /// `isNormalized()`.
    pub(crate) fn is_normalized(&self) -> bool {
        match self {
            Value::SetEnum(s) => s.norm.get().is_some(),
            Value::Record(r) => r.is_normalized(),
            Value::FcnRcd(f) => f.is_normalized(),
            Value::FcnLambda(f) => f.converted.get().is_some_and(|r| r.is_normalized()),
            // `SetPredValue` replaces `inVal` with the converted set, so once converted it
            // reports that set's flag (`SetPredValue.java:220-222`).
            Value::SetPred(s) => match s.converted.get() {
                Some(set) => set.norm.get().is_some(),
                None => s.inner.is_normalized(),
            },
            // A cache that exists was deep-normalised; without one these classes report
            // their operands', and SetCup/Subset/Union report false.
            Value::SetCap(s) => match s.converted.get() {
                Some(set) => set.norm.get().is_some(),
                None => s.left.is_normalized() && s.right.is_normalized(),
            },
            Value::SetDiff(s) => match s.converted.get() {
                Some(set) => set.norm.get().is_some(),
                None => s.left.is_normalized(),
            },
            Value::SetOfFcns(s) => match s.converted.get() {
                Some(set) => set.norm.get().is_some(),
                None => s.domain.is_normalized() && s.range.is_normalized(),
            },
            Value::SetOfRcds(s) => match s.converted.get() {
                Some(set) => set.norm.get().is_some(),
                None => s.sets.iter().all(Value::is_normalized),
            },
            Value::SetOfTuples(s) => match s.converted.get() {
                Some(set) => set.norm.get().is_some(),
                None => s.sets.iter().all(Value::is_normalized),
            },
            Value::SetCup(s) => s.converted.get().is_some_and(|set| set.norm.get().is_some()),
            Value::Subset(s) => s.converted.get().is_some_and(|set| set.norm.get().is_some()),
            Value::KSubset(s) => s.converted.get().is_some_and(|set| set.norm.get().is_some()),
            Value::Union(s) => s.converted.get().is_some_and(|set| set.norm.get().is_some()),
            _ => true,
        }
    }

    /// `Reducible`: only Interval and SetEnum; decides eager vs lazy set operators.
    pub(crate) fn is_reducible(&self) -> bool {
        matches!(self, Value::Interval(_) | Value::SetEnum(_))
    }

    /// `instanceof Enumerable`: the set classes. TLC tests the class, not whether the
    /// enumeration succeeds, so `SUBSET Nat` is enumerable and fails while enumerating.
    pub(crate) fn is_enumerable(&self) -> bool {
        matches!(
            self,
            Value::SetEnum(_)
                | Value::Interval(_)
                | Value::Subset(_)
                | Value::KSubset(_)
                | Value::SetOfFcns(_)
                | Value::SetOfRcds(_)
                | Value::SetOfTuples(_)
                | Value::Union(_)
                | Value::SetCup(_)
                | Value::SetCap(_)
                | Value::SetDiff(_)
                | Value::SetPred(_)
        )
    }

    /// `normalize()`: the in-place sort TLC runs before a value is compared or printed. Here
    /// it publishes the `OnceLock`, after which only the normalised form is read.
    pub(crate) fn normalize(&self, ev: &mut Ev<'_>) -> EvalResult<()> {
        match self {
            Value::Bool(_) | Value::Int(_) | Value::Str(_) | Value::Model(_) | Value::Interval(_) | Value::User(_) | Value::Undef => Ok(()),
            Value::Tuple(_) => Ok(()),
            Value::SetEnum(s) => order::normalized(ev, s).map(|_| ()),
            Value::Record(r) => order::record_normalized(ev, r).map(|_| ()),
            Value::FcnRcd(f) => f.normalize(ev).map(|_| ()),
            Value::FcnLambda(f) => match f.converted.get() {
                Some(r) => r.normalize(ev).map(|_| ()),
                None => Ok(()),
            },
            // A lazy set with a cache normalises the cache; without one it normalises the
            // operands it reads its own `isNormalized` from, and nothing else.
            Value::SetCup(s) => cached_or(ev, s.converted.get(), &[]),
            Value::Union(u) => cached_or(ev, u.converted.get(), &[]),
            Value::SetCap(s) | Value::SetDiff(s) => cached_or(ev, s.converted.get(), &[s.left.clone(), s.right.clone()]),
            Value::Subset(s) => cached_or(ev, s.converted.get(), &[s.base.clone()]),
            Value::KSubset(s) => cached_or(ev, s.converted.get(), &[s.base.clone()]),
            Value::SetOfFcns(s) => cached_or(ev, s.converted.get(), &[s.domain.clone(), s.range.clone()]),
            Value::SetOfRcds(s) => cached_or(ev, s.converted.get(), &s.sets),
            Value::SetOfTuples(s) => cached_or(ev, s.converted.get(), &s.sets),
            Value::SetPred(s) => match s.converted.get() {
                Some(set) => order::normalized(ev, set).map(|_| ()),
                None => s.inner.normalize(ev),
            },
        }
    }

    /// `deepNormalize()`: done before a state is fingerprinted / shared.
    pub(crate) fn deep_normalize(&self, ev: &mut Ev<'_>) -> EvalResult<()> {
        match self {
            Value::Bool(_) | Value::Int(_) | Value::Str(_) | Value::Model(_) | Value::Interval(_) | Value::User(_) | Value::Undef => Ok(()),
            // Elements first, then the set itself (`SetEnumValue.java:307-319`).
            Value::SetEnum(set) => {
                for e in set.built.iter() {
                    e.deep_normalize(ev)?;
                }
                order::normalized(ev, set).map(|_| ())
            }
            Value::Tuple(t) => {
                for e in t.elems.iter() {
                    e.deep_normalize(ev)?;
                }
                Ok(())
            }
            // The snapshot's values, then the record / function itself
            // (`RecordValue.java:653-666`, `FcnRcdValue.java:751-764`).
            Value::Record(r) => {
                for v in r.snapshot().1.iter() {
                    v.deep_normalize(ev)?;
                }
                order::record_normalized(ev, r).map(|_| ())
            }
            Value::FcnRcd(f) => {
                for v in f.snapshot().1.iter() {
                    v.deep_normalize(ev)?;
                }
                f.normalize(ev).map(|_| ())
            }
            Value::FcnLambda(f) => match f.converted.get() {
                Some(r) => Value::FcnRcd(r.clone()).deep_normalize(ev),
                None => {
                    for ex in f.excepts.iter() {
                        ex.value.deep_normalize(ev)?;
                        for p in ex.path.iter() {
                            p.deep_normalize(ev)?;
                        }
                    }
                    for (_, _, domain) in f.params.iter() {
                        domain.deep_normalize(ev)?;
                    }
                    Ok(())
                }
            },
            // A lazy set descends into its operands and leaves itself lazy
            // (`SetCupValue.java:150-160`).
            Value::SetCup(s) | Value::SetCap(s) | Value::SetDiff(s) => {
                s.left.deep_normalize(ev)?;
                s.right.deep_normalize(ev)
            }
            Value::Subset(s) => s.base.deep_normalize(ev),
            Value::KSubset(s) => s.base.deep_normalize(ev),
            Value::Union(u) => u.set.deep_normalize(ev),
            Value::SetOfFcns(s) => {
                s.domain.deep_normalize(ev)?;
                s.range.deep_normalize(ev)
            }
            Value::SetOfRcds(s) => s.sets.iter().try_for_each(|v| v.deep_normalize(ev)),
            Value::SetOfTuples(s) => s.sets.iter().try_for_each(|v| v.deep_normalize(ev)),
            Value::SetPred(s) => match s.converted.get() {
                Some(set) => Value::SetEnum(set.clone()).deep_normalize(ev),
                None => s.inner.deep_normalize(ev),
            },
        }
    }

    /// `permute(MVPerm)`.
    pub(crate) fn permute(&self, ev: &mut Ev<'_>, perm: &crate::symmetry::MvPerm) -> EvalResult<Value> {
        todo!()
    }
}

/// `new SetEnumValue(vals, isNorm)` as an `Arc`.
pub(crate) fn set_enum_arc(elems: Vec<Value>, is_norm: bool) -> Arc<SetEnumValue> {
    let elems = elems.into_boxed_slice();
    let norm = OnceLock::new();
    if is_norm {
        let _ = norm.set(elems.clone());
    }
    Arc::new(SetEnumValue { built: elems, norm })
}

/// `normalize()` of a lazy set: the cached SetEnum if it has one, else the given operands.
fn cached_or(ev: &mut Ev<'_>, cached: Option<&Arc<SetEnumValue>>, operands: &[Value]) -> EvalResult<()> {
    match cached {
        Some(set) => order::normalized(ev, set).map(|_| ()),
        None => operands.iter().try_for_each(|v| v.normalize(ev)),
    }
}

/// `tlc2.value.impl.ValueVec`: the array a set class fills element by element. The single
/// owner of "a set built one element at a time holds at most `TLCGlobals.setBound` of them":
/// the array refuses to grow past the bound, which an initial capacity may still exceed
/// (`ValueVec.java:90-108`), so `SUBSET S` of a 20-element S is built without complaint.
pub(crate) struct ValueVec {
    elems: Vec<Value>,
    capacity: usize,
    bound: usize,
}

impl ValueVec {
    /// `new ValueVec()`.
    pub(crate) fn new(ev: &Ev<'_>) -> ValueVec {
        ValueVec::with_capacity(ev, 10)
    }

    /// `new ValueVec(initialCapacity)`.
    pub(crate) fn with_capacity(ev: &Ev<'_>, capacity: usize) -> ValueVec {
        ValueVec { elems: Vec::new(), capacity, bound: ev.tool().globals().max_set_size.max(0) as usize }
    }

    /// `addElement(val)`, which grows the array when it is full.
    pub(crate) fn push(&mut self, value: Value) -> EvalResult<()> {
        if self.elems.len() == self.capacity {
            if self.capacity >= self.bound {
                let bound = self.bound;
                return Err(EvalError::runtime(
                    tlars_msg::Ec::GENERAL,
                    format!("Attempted to construct a set with too many elements (>{bound})."),
                ));
            }
            let doubled = (self.capacity as i64 * 2).min(i64::from(i32::MAX) - 8);
            self.capacity = doubled.max(self.elems.len() as i64 + 1).min(self.bound as i64) as usize;
        }
        self.elems.push(value);
        Ok(())
    }

    pub(crate) fn into_vec(self) -> Vec<Value> {
        self.elems
    }
}
