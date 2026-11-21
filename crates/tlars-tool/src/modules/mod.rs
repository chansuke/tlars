//! Native standard-module operators (`tlc2.module.*`), one registry.
//!
//! TLC binds a module's definitions to the public static methods of the Java class named
//! like the module (`SpecProcessor.processModuleOverrides`), mapping method names to TLA+
//! names through `TLARegistry`. Modules are processed in `getModuleNodes()` order and a
//! module's definitions include the ones it extends, so with `EXTENDS Integers` the `+` of
//! Naturals ends up bound to `Integers.Plus`. Overrides replace bodies for *evaluation
//! only*; SANY still level-checks the TLA+ text. User `.class` overrides are not supported
//! in v1 (see DESIGN.md).

mod finite_sets;
mod naturals;
mod sequences;
pub(crate) mod tlc;
mod tlc_get_set;

pub(crate) use naturals::eval_exception;

use crate::ctx::Ctx;
use crate::error::{EvalError, EvalResult};
use crate::eval::{EvalControl, Ev};
use crate::state::StateRef;
use crate::value::Value;
use tlars_sany::graph::ExprOrOpArg;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct NativeId(u16);

pub(crate) type StrictFn = fn(&mut Ev<'_>, &[Value]) -> EvalResult<Value>;
/// `@Evaluation`-style: receives unevaluated arguments (TLCGet, Assert, ...).
pub(crate) type LazyFn = fn(&mut Ev<'_>, &[ExprOrOpArg], &Ctx, StateRef<'_>, StateRef<'_>, EvalControl) -> EvalResult<Value>;

#[derive(Clone, Copy)]
pub(crate) enum NativeImpl {
    Strict(StrictFn),
    Lazy(LazyFn),
}

/// A Java parameter type, for the `ClassCastException` `MethodHandle.invoke` raises when an
/// argument has another class.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum JavaParam {
    Value,
    IntValue,
}

pub(crate) struct NativeOp {
    /// The Java class, which is the module it overrides.
    pub(crate) module: &'static str,
    pub(crate) tla_name: &'static str,
    pub(crate) params: &'static [JavaParam],
    pub(crate) imp: NativeImpl,
    /// `Method.toString()`, as TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE prints it.
    pub(crate) signature: &'static str,
    /// `@Evaluation(minLevel = ...)` / `@TLAPlusOperator(minLevel = ...)`, which
    /// `getLevelBound` takes as the level of an application. It is what keeps `TLCGet` out of
    /// constant folding (`SymbolNodeValueLookupProvider.java:270-276`).
    pub(crate) min_level: u8,
}

use JavaParam::{IntValue as I, Value as V};

macro_rules! natives {
    ($( $class:literal $name:literal ($($p:ident),*) $kind:ident $imp:expr, $sig:literal $(, min_level $lvl:literal)?; )*) => {
        pub(crate) static REGISTRY: &[NativeOp] = &[
            $( NativeOp {
                module: $class,
                tla_name: $name,
                params: &[$($p),*],
                imp: NativeImpl::$kind($imp),
                signature: $sig,
                min_level: 0 $(+ $lvl)?,
            }, )*
        ];
    };
}

natives! {
    "Naturals" "Nat" () Strict naturals::nat, "public static tlc2.value.impl.Value tlc2.module.Naturals.Nat()";
    "Naturals" "+" (I, I) Strict naturals::plus, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Plus(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "-" (I, I) Strict naturals::minus, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Minus(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "*" (I, I) Strict naturals::times, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Times(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "<" (V, V) Strict naturals::lt, "public static tlc2.value.IBoolValue tlc2.module.Naturals.LT(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Naturals" "\\leq" (V, V) Strict naturals::le, "public static tlc2.value.IBoolValue tlc2.module.Naturals.LE(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Naturals" ">" (V, V) Strict naturals::gt, "public static tlc2.value.IBoolValue tlc2.module.Naturals.GT(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Naturals" "\\geq" (V, V) Strict naturals::geq_naturals, "public static tlc2.value.IBoolValue tlc2.module.Naturals.GEQ(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Naturals" ".." (I, I) Strict naturals::dot_dot, "public static tlc2.value.impl.IntervalValue tlc2.module.Naturals.DotDot(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "\\div" (I, I) Strict naturals::divide_naturals, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Divide(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "%" (I, I) Strict naturals::modulo, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Mod(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "^" (I, I) Strict naturals::expt, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Expt(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "Int" () Strict naturals::int, "public static tlc2.value.impl.Value tlc2.module.Integers.Int()";
    "Integers" "Nat" () Strict naturals::nat, "public static tlc2.value.impl.Value tlc2.module.Integers.Nat()";
    "Integers" "+" (I, I) Strict naturals::plus, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Plus(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "-" (I, I) Strict naturals::minus, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Minus(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "*" (I, I) Strict naturals::times, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Times(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "<" (V, V) Strict naturals::lt, "public static tlc2.value.IBoolValue tlc2.module.Integers.LT(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Integers" "\\leq" (V, V) Strict naturals::le, "public static tlc2.value.IBoolValue tlc2.module.Integers.LE(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Integers" ">" (V, V) Strict naturals::gt, "public static tlc2.value.impl.BoolValue tlc2.module.Integers.GT(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Integers" "\\geq" (V, V) Strict naturals::geq_integers, "public static tlc2.value.IBoolValue tlc2.module.Integers.GEQ(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Integers" ".." (I, I) Strict naturals::dot_dot, "public static tlc2.value.impl.IntervalValue tlc2.module.Integers.DotDot(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "-." (I) Strict naturals::neg, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Neg(tlc2.value.impl.IntValue)";
    "Integers" "\\div" (I, I) Strict naturals::divide_integers, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Divide(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "%" (I, I) Strict naturals::modulo, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Mod(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "^" (I, I) Strict naturals::expt, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Expt(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Sequences" "Seq" (V) Strict sequences::seq, "public static tlc2.value.impl.Value tlc2.module.Sequences.Seq(tlc2.value.impl.Value)";
    "Sequences" "Len" (V) Strict sequences::len, "public static tlc2.value.impl.IntValue tlc2.module.Sequences.Len(tlc2.value.impl.Value)";
    "Sequences" "Head" (V) Strict sequences::head, "public static tlc2.value.impl.Value tlc2.module.Sequences.Head(tlc2.value.impl.Value)";
    "Sequences" "Tail" (V) Strict sequences::tail, "public static tlc2.value.impl.Value tlc2.module.Sequences.Tail(tlc2.value.impl.Value)";
    "Sequences" "Append" (V, V) Strict sequences::append, "public static tlc2.value.impl.Value tlc2.module.Sequences.Append(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Sequences" "\\o" (V, V) Strict sequences::concat, "public static tlc2.value.impl.Value tlc2.module.Sequences.Concat(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Sequences" "SubSeq" (V, V, V) Strict sequences::sub_seq, "public static tlc2.value.impl.Value tlc2.module.Sequences.SubSeq(tlc2.value.impl.Value,tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Sequences" "SelectSeq" (V, V) Lazy sequences::select_seq, "public static tlc2.value.impl.Value tlc2.module.Sequences.SelectSeq(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "FiniteSets" "IsFiniteSet" (V) Strict finite_sets::is_finite_set, "public static tlc2.value.IBoolValue tlc2.module.FiniteSets.IsFiniteSet(tlc2.value.impl.Value)";
    "FiniteSets" "Cardinality" (V) Strict finite_sets::cardinality, "public static tlc2.value.impl.IntValue tlc2.module.FiniteSets.Cardinality(tlc2.value.impl.Value)";
    "TLC" "Print" (V, V) Strict tlc::print, "public static tlc2.value.impl.Value tlc2.module.TLC.Print(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "TLC" "PrintT" (V) Strict tlc::print_t, "public static tlc2.value.impl.Value tlc2.module.TLC.PrintT(tlc2.value.impl.Value)";
    "TLC" "ToString" (V) Strict tlc::to_string, "public static tlc2.value.impl.Value tlc2.module.TLC.ToString(tlc2.value.impl.Value)";
    "TLC" "Assert" (V, V) Strict tlc::assert, "public static tlc2.value.impl.Value tlc2.module.TLC.Assert(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "TLC" "Permutations" (V) Strict tlc::permutations, "public static tlc2.value.impl.Value tlc2.module.TLC.Permutations(tlc2.value.impl.Value)";
    "TLC" ":>" (V, V) Strict tlc::make_fcn, "public static tlc2.value.impl.Value tlc2.module.TLC.MakeFcn(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "TLC" "@@" (V, V) Strict tlc::combine_fcn, "public static tlc2.value.impl.Value tlc2.module.TLC.CombineFcn(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "TLC" "TLCGet" (V) Lazy tlc_get_set::tlc_get, "public static tlc2.value.impl.Value tlc2.module.TLCGetSet.TLCGetEval(tlc2.tool.impl.Tool,tla2sany.semantic.ExprOrOpArgNode[],tlc2.util.Context,tlc2.tool.TLCState,tlc2.tool.TLCState,int,tlc2.tool.coverage.CostModel)", min_level 1;
    "TLC" "TLCSet" (V, V) Strict tlc_get_set::tlc_set, "public static tlc2.value.impl.Value tlc2.module.TLCGetSet.TLCSet(tlc2.value.impl.Value,tlc2.value.impl.Value)";
}

/// `@Evaluation(minLevel = ...)` / `@TLAPlusOperator(minLevel = ...)` of the overrides tlars
/// does not implement yet. Every other method in `tlc2.module` keeps the annotation's default
/// of 0, so only these appear here: a level of 1 or more is what keeps a definition out of
/// `processConstantDefns` (`SymbolNodeValueLookupProvider.java:270-276`).
static UNIMPLEMENTED_MIN_LEVELS: &[(&str, &str, u8)] = &[
    // `TLCExt.java:100`, `LevelConstants.ActionLevel`.
    ("TLCExt", "PickSuccessor", 2),
    // `TLCExt.java:198`, `TLCExt.java:210`.
    ("TLCExt", "CounterExample", 1),
    ("TLCExt", "Trace", 1),
    // `_Possible.java:52`, `LevelConstants.VariableLevel`.
    ("_Possible", "_Counts", 1),
    // `_JsonTrace.java:39`, `_TLCTrace.java:87`.
    ("_JsonTrace", "_TLCState", 1),
    ("_TLCTrace", "_TLCState", 1),
];

/// The `minLevel` of the Java override of `module!tla_name`, 0 where the annotation leaves
/// the default or the definition has no override at all.
pub(crate) fn unimplemented_min_level(module: &str, tla_name: &str) -> u8 {
    UNIMPLEMENTED_MIN_LEVELS
        .iter()
        .find(|(m, n, _)| *m == module && *n == tla_name)
        .map_or(0, |&(_, _, level)| level)
}

/// The `tlc2.module` classes. A module with one of these names has its own definitions
/// bound to unimplemented natives first and then to [`REGISTRY`]'s, so a definition TLC
/// overrides in Java is never evaluated from its TLA+ body.
pub(crate) const JAVA_MODULES: &[&str] = &[
    "AnySet", "Bags", "FiniteSets", "Integers", "Json", "Naturals", "Randomization", "Sequences", "Strings", "TLC",
    "TLCEval", "TLCExt", "TLCGetSet", "TransitiveClosure", "_JsonTrace", "_Possible", "_TLCTrace",
];

impl NativeId {
    pub(crate) fn of_module(module: &str, tla_name: &str) -> Option<NativeId> {
        REGISTRY.iter().position(|op| op.module == module && op.tla_name == tla_name).map(|i| NativeId(i as u16))
    }

    pub(crate) fn op(self) -> &'static NativeOp {
        &REGISTRY[self.0 as usize]
    }
}

/// Whether this native receives its arguments evaluated.
pub(crate) fn imp(id: NativeId) -> NativeImpl {
    id.op().imp
}

/// The Java method's parameter count.
pub(crate) fn arity(id: NativeId) -> usize {
    id.op().params.len()
}

/// `EvaluatingValue.getMinLevel()` / `MethodValue.getMinLevel()`.
pub(crate) fn min_level(id: NativeId) -> u8 {
    id.op().min_level
}

/// `EvaluatingValue.eval`'s `catch (Throwable e)` (`EvaluatingValue.java:92-95`): where
/// `MethodValue` lets an `EvalException` the method raised on purpose through, an
/// `@Evaluation` override wraps *every* failure, so even `TLCGet(1) was undefined` reaches
/// the spec inside `TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE`.
pub(crate) fn wrap_evaluating(ev: &Ev<'_>, module: &str, tla_name: &str, e: EvalError) -> EvalError {
    use crate::error::EvalErrorKind::{StackOverflow, Unimplemented};
    if matches!(e.kind, Unimplemented | StackOverflow) {
        return e;
    }
    let signature = NativeId::of_module(module, tla_name).expect("a registered native").op().signature;
    let message = ev.out().text(tlars_msg::Ec::TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE, &[signature, &e.message]);
    EvalError::runtime(tlars_msg::Ec::TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE, message)
}

/// `MethodValue.eval(args)`: the arguments are cast to the method's parameter types first.
pub(crate) fn apply(ev: &mut Ev<'_>, id: NativeId, args: &[Value]) -> EvalResult<Value> {
    let op = id.op();
    for (arg, param) in args.iter().zip(op.params) {
        if *param == JavaParam::IntValue && !matches!(arg, Value::Int(_)) {
            let cause = format!("Cannot cast tlc2.value.impl.{} to tlc2.value.impl.IntValue", arg.java_class());
            let message = ev.out().text(tlars_msg::Ec::TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE, &[op.signature, &cause]);
            return Err(EvalError::runtime(tlars_msg::Ec::TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE, message));
        }
    }
    match op.imp {
        NativeImpl::Strict(f) => f(ev, args).map_err(|e| wrap(ev, op, e)),
        NativeImpl::Lazy(_) => unreachable!("lazy natives are applied with their argument expressions"),
    }
}

/// `MethodValue.eval`'s catch (`MethodValue.java:147-169`): an `EvalException` the method
/// raised on purpose passes through, and anything else is rewrapped as
/// `TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE` naming the Java method. A construct tlars does
/// not implement is not one of TLC's failures, so it is not wrapped either.
fn wrap(ev: &Ev<'_>, op: &NativeOp, e: EvalError) -> EvalError {
    use crate::error::EvalErrorKind::{Eval, StackOverflow, Unimplemented};
    if matches!(e.kind, Eval | Unimplemented | StackOverflow) {
        return e;
    }
    let message = ev.out().text(tlars_msg::Ec::TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE, &[op.signature, &e.message]);
    EvalError::runtime(tlars_msg::Ec::TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE, message)
}
