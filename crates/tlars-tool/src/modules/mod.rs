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
mod tlc;

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
}

use JavaParam::{IntValue as I, Value as V};

macro_rules! natives {
    ($( $class:literal $name:literal ($($p:ident),*) $imp:expr, $sig:literal; )*) => {
        pub(crate) static REGISTRY: &[NativeOp] = &[
            $( NativeOp { module: $class, tla_name: $name, params: &[$($p),*], imp: NativeImpl::Strict($imp), signature: $sig }, )*
        ];
    };
}

natives! {
    "Naturals" "Nat" () naturals::nat, "public static tlc2.value.impl.Value tlc2.module.Naturals.Nat()";
    "Naturals" "+" (I, I) naturals::plus, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Plus(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "-" (I, I) naturals::minus, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Minus(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "*" (I, I) naturals::times, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Times(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "<" (V, V) naturals::lt, "public static tlc2.value.IBoolValue tlc2.module.Naturals.LT(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Naturals" "\\leq" (V, V) naturals::le, "public static tlc2.value.IBoolValue tlc2.module.Naturals.LE(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Naturals" ">" (V, V) naturals::gt, "public static tlc2.value.IBoolValue tlc2.module.Naturals.GT(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Naturals" "\\geq" (V, V) naturals::geq_naturals, "public static tlc2.value.IBoolValue tlc2.module.Naturals.GEQ(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Naturals" ".." (I, I) naturals::dot_dot, "public static tlc2.value.impl.IntervalValue tlc2.module.Naturals.DotDot(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "\\div" (I, I) naturals::divide_naturals, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Divide(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "%" (I, I) naturals::modulo, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Mod(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Naturals" "^" (I, I) naturals::expt, "public static tlc2.value.impl.IntValue tlc2.module.Naturals.Expt(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "Int" () naturals::int, "public static tlc2.value.impl.Value tlc2.module.Integers.Int()";
    "Integers" "Nat" () naturals::nat, "public static tlc2.value.impl.Value tlc2.module.Integers.Nat()";
    "Integers" "+" (I, I) naturals::plus, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Plus(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "-" (I, I) naturals::minus, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Minus(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "*" (I, I) naturals::times, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Times(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "<" (V, V) naturals::lt, "public static tlc2.value.IBoolValue tlc2.module.Integers.LT(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Integers" "\\leq" (V, V) naturals::le, "public static tlc2.value.IBoolValue tlc2.module.Integers.LE(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Integers" ">" (V, V) naturals::gt, "public static tlc2.value.impl.BoolValue tlc2.module.Integers.GT(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Integers" "\\geq" (V, V) naturals::geq_integers, "public static tlc2.value.IBoolValue tlc2.module.Integers.GEQ(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "Integers" ".." (I, I) naturals::dot_dot, "public static tlc2.value.impl.IntervalValue tlc2.module.Integers.DotDot(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "-." (I) naturals::neg, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Neg(tlc2.value.impl.IntValue)";
    "Integers" "\\div" (I, I) naturals::divide_integers, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Divide(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "%" (I, I) naturals::modulo, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Mod(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "Integers" "^" (I, I) naturals::expt, "public static tlc2.value.impl.IntValue tlc2.module.Integers.Expt(tlc2.value.impl.IntValue,tlc2.value.impl.IntValue)";
    "FiniteSets" "IsFiniteSet" (V) finite_sets::is_finite_set, "public static tlc2.value.IBoolValue tlc2.module.FiniteSets.IsFiniteSet(tlc2.value.impl.Value)";
    "FiniteSets" "Cardinality" (V) finite_sets::cardinality, "public static tlc2.value.impl.IntValue tlc2.module.FiniteSets.Cardinality(tlc2.value.impl.Value)";
    "TLC" "Print" (V, V) tlc::print, "public static tlc2.value.impl.Value tlc2.module.TLC.Print(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "TLC" "PrintT" (V) tlc::print_t, "public static tlc2.value.impl.Value tlc2.module.TLC.PrintT(tlc2.value.impl.Value)";
    "TLC" "ToString" (V) tlc::to_string, "public static tlc2.value.impl.Value tlc2.module.TLC.ToString(tlc2.value.impl.Value)";
    "TLC" "Assert" (V, V) tlc::assert, "public static tlc2.value.impl.Value tlc2.module.TLC.Assert(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "TLC" ":>" (V, V) tlc::make_fcn, "public static tlc2.value.impl.Value tlc2.module.TLC.MakeFcn(tlc2.value.impl.Value,tlc2.value.impl.Value)";
    "TLC" "@@" (V, V) tlc::combine_fcn, "public static tlc2.value.impl.Value tlc2.module.TLC.CombineFcn(tlc2.value.impl.Value,tlc2.value.impl.Value)";
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
