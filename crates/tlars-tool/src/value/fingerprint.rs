//! FP64 per-kind encoding (`fingerPrint(long)` of every value class): the only place that
//! decides which bytes a value contributes.
//!
//! - Bool: `BOOL, 't'|'f'`; Int: `INT, int32`; String: `STRING, len, low byte per char`;
//!   ModelValue: `MODEL, tok`.
//! - SetEnum: `SETENUM, size, elems` in normalised order; Interval hashes as a SetEnum.
//! - FcnRcd: `FCNRCD, n, (dom, val)*` (interval domain writes `INT, i`); Tuple as FCNRCD with
//!   keys `1..n`; Record as FCNRCD with STRING keys in token order.
//! - Lazy sets / FcnLambda: converted first.
//! So equal values fingerprint equally whatever their representation, and `<<>>` differs
//! from `{}`.

use super::{Value, ValueKind};
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use crate::fp64::Fp;
use tlars_sany::Atom;

impl Value {
    pub(crate) fn fingerprint(&self, ev: &mut Ev<'_>, fp: Fp) -> EvalResult<Fp> {
        let f = &ev.tool().fp64;
        match self {
            Value::Bool(b) => {
                let fp = f.extend_byte(fp, ValueKind::Bool as u8);
                Ok(f.extend_byte(fp, if *b { b't' } else { b'f' }))
            }
            Value::Int(n) => {
                let fp = f.extend_byte(fp, ValueKind::Int as u8);
                Ok(f.extend_i32(fp, *n))
            }
            Value::Str(a) => Ok(string_fp(ev, fp, *a)),
            Value::Model(mv) => {
                // `UniqueString.fingerPrint`: the intern token, not the characters.
                let fp = f.extend_byte(fp, ValueKind::Model as u8);
                Ok(f.extend_i32(fp, mv.name.tok()))
            }
            Value::Interval(iv) => {
                let size = iv.size(ev)?;
                let f = &ev.tool().fp64;
                let mut fp = f.extend_byte(fp, ValueKind::SetEnum as u8);
                fp = f.extend_i32(fp, size);
                for i in 0..size {
                    fp = f.extend_byte(fp, ValueKind::Int as u8);
                    fp = f.extend_i32(fp, iv.lo + i);
                }
                Ok(fp)
            }
            Value::SetEnum(set) => {
                let elems = super::order::normalized(ev, set)?.to_vec();
                let f = &ev.tool().fp64;
                let mut fp = f.extend_byte(fp, ValueKind::SetEnum as u8);
                fp = f.extend_i32(fp, elems.len() as i32);
                for e in elems.iter() {
                    fp = e.fingerprint(ev, fp)?;
                }
                Ok(fp)
            }
            // A tuple is a function with the keys `1..n`.
            Value::Tuple(t) => {
                let elems = t.elems.clone();
                let f = &ev.tool().fp64;
                let mut fp = f.extend_byte(fp, ValueKind::FcnRcd as u8);
                fp = f.extend_i32(fp, elems.len() as i32);
                for (i, e) in elems.iter().enumerate() {
                    let f = &ev.tool().fp64;
                    fp = f.extend_byte(fp, ValueKind::Int as u8);
                    fp = f.extend_i32(fp, i as i32 + 1);
                    fp = e.fingerprint(ev, fp)?;
                }
                Ok(fp)
            }
            // A record is a function with STRING keys, in token order.
            Value::Record(r) => {
                let (names, values) = super::order::record_normalized(ev, r)?;
                let (names, values) = (names.to_vec(), values.to_vec());
                let f = &ev.tool().fp64;
                let mut fp = f.extend_byte(fp, ValueKind::FcnRcd as u8);
                fp = f.extend_i32(fp, names.len() as i32);
                for (name, value) in names.iter().zip(&values) {
                    fp = string_fp(ev, fp, *name);
                    fp = value.fingerprint(ev, fp)?;
                }
                Ok(fp)
            }
            Value::FcnRcd(fcn) => {
                let (domain, values) = fcn.normalize(ev)?;
                let (domain, values) = (domain.map(<[Value]>::to_vec), values.to_vec());
                let interval = fcn.interval();
                let f = &ev.tool().fp64;
                let mut fp = f.extend_byte(fp, ValueKind::FcnRcd as u8);
                fp = f.extend_i32(fp, values.len() as i32);
                for (i, value) in values.iter().enumerate() {
                    match (&domain, interval) {
                        (Some(domain), _) => fp = domain[i].fingerprint(ev, fp)?,
                        (None, Some(iv)) => {
                            let f = &ev.tool().fp64;
                            fp = f.extend_byte(fp, ValueKind::Int as u8);
                            fp = f.extend_i32(fp, iv.lo + i as i32);
                        }
                        (None, None) => unreachable!("a function record has an explicit or an interval domain"),
                    }
                    fp = value.fingerprint(ev, fp)?;
                }
                Ok(fp)
            }
            Value::FcnLambda(l) => {
                let fcn = l.to_fcn_rcd(ev)?;
                Value::FcnRcd(fcn).fingerprint(ev, fp)
            }
            Value::Subset(_)
            | Value::KSubset(_)
            | Value::SetOfFcns(_)
            | Value::SetOfRcds(_)
            | Value::SetOfTuples(_)
            | Value::Union(_)
            | Value::SetCup(_)
            | Value::SetCap(_)
            | Value::SetDiff(_)
            | Value::SetPred(_) => {
                let set = self.convert_and_cache(ev)?;
                Value::SetEnum(set).fingerprint(ev, fp)
            }
            // `Value.fingerPrint` (`Value.java:265-280`): the classes that do not override it
            // are the ones a variable may not hold.
            Value::User(_) | Value::Undef => {
                let shown = self.ppr(ev)?;
                Err(EvalError::detailed(format!("TLC has found a state in which the value of a variable contains {shown}"), None))
            }
        }
    }
}

/// `StringValue.fingerPrint`, which record field names reuse: the UTF-16 length, then the low
/// byte of each unit.
fn string_fp(ev: &Ev<'_>, fp: Fp, a: Atom) -> Fp {
    let text = ev.tool().graph().interner().text(a);
    let f = &ev.tool().fp64;
    let fp = f.extend_byte(fp, ValueKind::String as u8);
    let fp = f.extend_i32(fp, text.encode_utf16().count() as i32);
    f.extend_str(fp, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{IntervalValue, ModelValue};
    use std::sync::Arc;

    /// Replays `tests/golden/fingerprint.txt`, produced by TLC's own value classes through
    /// `tools/golden/FingerprintGolden.java`. The golden carries each value's printed form,
    /// so a list that drifted out of step with the Java one fails on the label instead of
    /// comparing fingerprints of two different values.
    #[test]
    fn matches_tlc_fingerprint() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/fingerprint.txt");
        let text = std::fs::read_to_string(path).expect("golden file");
        let mut lines = text.lines().peekable();
        let mut toks: Vec<(i32, String)> = Vec::new();
        while let Some(line) = lines.peek().and_then(|l| l.strip_prefix("tok\t")) {
            let (tok, name) = line.split_once('\t').expect("tok <n> <name>");
            toks.push((tok.parse().expect("tok"), name.to_string()));
            lines.next();
        }
        let mut poly = usize::MAX;
        let mut fixture = None;
        let mut index = 0;
        let mut checked = 0;
        for line in lines {
            let mut f = line.splitn(3, '\t');
            let (p, want, label) = (f.next().expect("poly"), f.next().expect("fp"), f.next().expect("label"));
            let p: usize = p.parse().expect("poly index");
            if p != poly {
                poly = p;
                fixture = Some(Fixture::new(p, &toks));
                index = 0;
            }
            let (shown, fp) = fixture.as_ref().expect("a poly line precedes its values").at(index);
            assert_eq!(shown, label, "value {index} of poly {p}");
            assert_eq!(format!("{:x}", fp.raw()), want, "value {index} of poly {p} ({label})");
            index += 1;
            checked += 1;
        }
        assert!(checked > 0, "the golden file has values");
    }

    /// A `Tool` over the smallest module SANY accepts, which is all the value methods need.
    struct Fixture {
        tool: crate::tool::Tool,
        model_values: Vec<ModelValue>,
    }

    impl Fixture {
        fn new(poly: usize, toks: &[(i32, String)]) -> Fixture {
            let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
            let interner = Arc::new(tlars_sany::Interner::new());
            // A model value fingerprints as its intern token, so each name is interned at the
            // position TLC gave it, with fillers in between.
            let mut names = Vec::new();
            for (tok, name) in toks {
                while interner.len() as i32 + 1 < *tok {
                    interner.intern(&format!("\u{0}filler{}", interner.len()));
                }
                names.push(interner.intern(name));
                assert_eq!(names.last().expect("just interned").tok(), *tok, "{name}");
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
                out: &out,
            })
            .expect("the fixture module builds a tool");
            let model_values =
                names.iter().enumerate().map(|(i, &a)| ModelValue::new(a, i as u16, &interner.text(a))).collect();
            Fixture { tool, model_values }
        }

        /// The printed form and fingerprint of value `index`. The list is rebuilt per call so
        /// each value is fingerprinted from its constructed state, as the Java golden does
        /// with a fresh object.
        fn at(&self, index: usize) -> (String, Fp) {
            let out = report();
            let mut locals = crate::eval::WorkerLocals::new(0, 4);
            let mut ev = self.tool.ev(&mut locals, &out);
            let values = golden_values(&mut ev, &self.model_values);
            let v = values[index].clone();
            let shown = v.to_tlc_string(&mut ev).expect("printable");
            let start = ev.tool().fp64.new_fp();
            let fp = v.fingerprint(&mut ev, start).expect("fingerprintable");
            (shown, fp)
        }
    }

    fn globals(poly: usize) -> crate::globals::TlcGlobals {
        crate::globals::TlcGlobals {
            tool_mode: false,
            workers: 1,
            check_deadlock: true,
            continue_after_violation: false,
            difftrace: false,
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
        }
    }

    fn report() -> tlars_msg::Reporter<'static> {
        tlars_msg::Reporter::new(
            Box::new(std::io::sink()),
            tlars_msg::ReporterConfig {
                tool_mode: false,
                warnings_enabled: false,
                suppressed: Vec::new(),
                as_errors: Vec::new(),
                debug: false,
                now: Box::new(|| "NOW".into()),
            },
        )
    }

    struct Silent;

    impl tlars_sany::SanyOut for Silent {
        fn log_line(&mut self, _text: &str) {}
        fn line(&mut self, _text: &str) {}
        fn err_line(&mut self, _text: &str) {}
    }

    /// The same list as `FingerprintGolden.values`, in the same order.
    fn golden_values(ev: &mut Ev<'_>, mv: &[ModelValue]) -> Vec<Value> {
        let int = Value::Int;
        let set = Value::set_enum;
        let tuple = Value::tuple;
        let s = |t: &str| Value::Str(atom(ev, t));
        let record = |names: &[&str], values: Vec<Value>| {
            Value::Record(Value::record_arc(names.iter().map(|n| atom(ev, n)).collect(), values, false))
        };
        let interval = |lo, hi| Value::Interval(IntervalValue { lo, hi });
        let fcn = |domain, values, is_norm| Value::FcnRcd(Value::fcn_rcd_arc(domain, values, is_norm));
        let fcn_interval = |lo, hi, values| Value::FcnRcd(Value::fcn_rcd_interval_arc(IntervalValue { lo, hi }, values));
        let mut values = vec![
            Value::Bool(true),
            Value::Bool(false),
            int(0),
            int(1),
            int(-1),
            int(255),
            int(256),
            int(i32::MAX),
            int(i32::MIN),
            s(""),
            s("a"),
            s("abc"),
            s("aa"),
            s("\u{100}A"),
            s("\u{0}A"),
            Value::Model(mv[0]),
            Value::Model(mv[2]),
            Value::Model(mv[3]),
            tuple(vec![]),
            tuple(vec![int(1)]),
            tuple(vec![int(1), s("a"), Value::Bool(true)]),
            tuple(vec![tuple(vec![int(1)]), set(vec![])]),
            record(&["bb", "a"], vec![int(1), int(2)]),
            record(&["a", "bb"], vec![int(2), int(1)]),
            record(&["zz", "a", "bb"], vec![set(vec![int(3), int(1)]), Value::Model(mv[1]), s("x")]),
            set(vec![]),
            set(vec![int(1)]),
            set(vec![int(3), int(1), int(2), int(1)]),
            Value::set_enum_normalized(vec![int(1), int(2), int(3)]),
            set(vec![s("b"), s("a")]),
            set(vec![Value::Model(mv[1]), Value::Model(mv[0])]),
            set(vec![Value::Bool(true), Value::Bool(false)]),
            set(vec![set(vec![int(2), int(1)]), set(vec![])]),
            interval(1, 3),
            interval(0, 0),
            interval(5, 4),
            interval(-2, 2),
            set(vec![int(2), int(1), int(3)]),
            fcn(vec![s("bb"), s("a")], vec![int(1), int(2)], false),
            fcn(vec![int(2), int(1)], vec![s("b"), s("a")], false),
            fcn_interval(1, 3, vec![int(7), int(8), int(9)]),
            fcn_interval(-1, 1, vec![int(7), int(8), int(9)]),
            fcn(vec![int(1), int(2), int(3)], vec![int(7), int(8), int(9)], true),
            crate::value::subset(set(vec![int(1), int(2)])),
            crate::value::lazy_cup(set(vec![int(1)]), set(vec![int(2), int(1)])),
            crate::value::lazy_cap(set(vec![int(1), int(2)]), set(vec![int(2), int(3)])),
            crate::value::lazy_diff(set(vec![int(1), int(2)]), set(vec![int(2)])),
            Value::Union(Arc::new(crate::value::UnionValue {
                set: set(vec![set(vec![int(1)]), set(vec![int(2), int(1)])]),
                converted: Default::default(),
            })),
            crate::value::set_of_fcns(set(vec![int(1), int(2)]), set(vec![Value::Bool(false), Value::Bool(true)])),
            crate::value::set_of_tuples(vec![set(vec![int(1), int(2)]), set(vec![s("a")])]),
        ];
        let names = vec![atom(ev, "a"), atom(ev, "bb")];
        let sets = vec![set(vec![int(1)]), set(vec![int(2), int(3)])];
        values.push(crate::value::set_of_rcds(ev, names, sets).expect("a set of records"));
        values
    }

    fn atom(ev: &Ev<'_>, s: &str) -> Atom {
        ev.tool().graph().interner().intern(s)
    }
}
