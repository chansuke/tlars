//! Native standard-module operators (`tlc2.module.*`), one registry.
//!
//! TLC binds a module's definitions to the public static methods of the Java class named
//! like the module (`SpecProcessor.processModuleOverrides`), mapping method names to TLA+
//! names through `TLARegistry`. Modules are processed in `getModuleNodes()` order and a
//! module's definitions include the ones it extends, so with `EXTENDS Integers` the `+` of
//! Naturals ends up bound to `Integers.Plus`. Overrides replace bodies for *evaluation
//! only*; SANY still level-checks the TLA+ text. User `.class` overrides are not supported
//! in v1 (see DESIGN.md).
//!
//! Which definitions have an override at all is [`java_overrides::JAVA_OVERRIDES`], read out
//! of the pinned Java by `xtask gen-fidelity`. [`REGISTRY`] is the part of it tlars
//! implements; the rest stops evaluation as unimplemented. Everything else, including the
//! other definitions of a module that does have a Java class, is an ordinary TLA+ definition.

mod any_set;
mod finite_sets;
mod java_overrides;
mod naturals;
mod possible;
mod sequences;
pub(crate) mod tlc;
mod tlc_eval;
mod tlc_get_set;

pub(crate) use naturals::eval_exception;

pub(crate) use java_overrides::{JAVA_OVERRIDES, JavaOverride};

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
    "_Possible" "_Counts" () Strict possible::counts, "public static tlc2.value.impl.Value tlc2.module._Possible.counts()", min_level 1;
    "AnySet" "ANY" () Strict any_set::any, "public static tlc2.value.impl.Value tlc2.module.AnySet.ANY()";
    "TLC" "Any" () Strict any_set::any, "public static tlc2.value.impl.Value tlc2.module.TLC.Any()";
    "TLC" "TLCEval" (V) Lazy tlc_eval::tlc_eval, "public static tlc2.value.impl.Value tlc2.module.TLCEval.tlcEval(tlc2.tool.impl.Tool,tla2sany.semantic.ExprOrOpArgNode[],tlc2.util.Context,tlc2.tool.TLCState,tlc2.tool.TLCState,int,tlc2.tool.coverage.CostModel)";
}

/// The Java override `processModuleOverrides` binds to `module!tla_name`, if any. A
/// definition with no entry here is an ordinary TLA+ definition, evaluated from its body,
/// whatever the module it lives in is named.
pub(crate) fn java_override(module: &str, tla_name: &str) -> Option<&'static JavaOverride> {
    JAVA_OVERRIDES.iter().find(|o| o.module == module && o.tla_name == tla_name)
}

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

#[cfg(test)]
mod tests {
    use super::{JAVA_OVERRIDES, REGISTRY, java_overrides::OverrideKind, java_override};

    /// Every native stands for a Java override TLC really has, with the same `minLevel` and,
    /// where TLC wraps the method in a `MethodValue`, the same parameter count. A native for
    /// an operator TLC evaluates from its TLA+ body would be a parity bug that no corpus case
    /// need cover.
    #[test]
    fn every_native_mirrors_a_java_override() {
        for op in REGISTRY {
            let o = java_override(op.module, op.tla_name)
                .unwrap_or_else(|| panic!("{}!{} is a native but TLC has no override for it", op.module, op.tla_name));
            assert_eq!(op.min_level, o.min_level, "{}!{} minLevel", op.module, op.tla_name);
            if o.kind == OverrideKind::Method {
                assert_eq!(op.params.len(), usize::from(o.params), "{}!{} arity of {}", op.module, op.tla_name, o.java);
            }
        }
    }

    /// The generated table is keyed by `(module, tla_name)`, so a repeated key would make
    /// `java_override` pick by position.
    #[test]
    fn java_overrides_are_unique() {
        for (i, o) in JAVA_OVERRIDES.iter().enumerate() {
            let first = JAVA_OVERRIDES.iter().position(|p| (p.module, p.tla_name) == (o.module, o.tla_name));
            assert_eq!(first, Some(i), "{}!{} appears twice", o.module, o.tla_name);
        }
    }
}
