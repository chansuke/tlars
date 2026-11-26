//! Property tests over generated values, std only: a seeded xorshift picks a value type, then
//! values of that type, so a failure names the seed that reproduces it.

use super::Value;
use super::golden::Fixture;
use crate::eval::Ev;
use std::cmp::Ordering;

const SEEDS: u64 = 300;

/// xorshift64*: small, deterministic, and good enough to pick shapes.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A value type: values of one type always compare without a cast error.
enum Ty {
    Int,
    Bool,
    Set(Box<Ty>),
    Tuple(Vec<Ty>),
}

fn ty(rng: &mut Rng, depth: u32) -> Ty {
    match if depth == 0 { rng.below(2) } else { rng.below(4) } {
        0 => Ty::Int,
        1 => Ty::Bool,
        2 => Ty::Set(Box::new(ty(rng, depth - 1))),
        _ => Ty::Tuple((0..1 + rng.below(3)).map(|_| ty(rng, depth - 1)).collect()),
    }
}

fn value(rng: &mut Rng, t: &Ty) -> Value {
    match t {
        Ty::Int => Value::Int(rng.below(7) as i32 - 3),
        Ty::Bool => Value::Bool(rng.below(2) == 0),
        Ty::Set(elem) => Value::set_enum((0..rng.below(4)).map(|_| value(rng, elem)).collect()),
        Ty::Tuple(elems) => Value::tuple(elems.iter().map(|e| value(rng, e)).collect()),
    }
}

fn shown(ev: &mut Ev<'_>, v: &Value) -> String {
    v.to_tlc_string(ev).expect("printable")
}

/// `ValueOutputStream` then `ValueInputStream`: the value read back equals the one written,
/// and writing it again gives the same bytes.
#[test]
fn the_value_stream_round_trips() {
    let fixture = Fixture::new(0, &[], &[]);
    fixture.with_ev(|ev| {
        for seed in 0..SEEDS {
            let mut rng = Rng::new(seed);
            let t = ty(&mut rng, 3);
            let v = value(&mut rng, &t);
            v.fingerprint(ev, crate::fp64::Fp::zero()).expect("fingerprintable");
            let bytes = super::stream::write(ev, &v).expect("writable");
            let back = super::stream::read_external(ev, &bytes).expect("readable");
            assert!(back.tlc_equals(ev, &v).expect("comparable"), "seed {seed}: {} read back as {}", shown(ev, &v), shown(ev, &back));
            assert_eq!(super::stream::write(ev, &back).expect("writable"), bytes, "seed {seed}: {}", shown(ev, &v));
        }
    });
}

/// `Value.compareTo` on values of one type is a total order that agrees with `equals`.
#[test]
fn comparison_is_a_total_order() {
    let fixture = Fixture::new(0, &[], &[]);
    fixture.with_ev(|ev| {
        for seed in 0..SEEDS {
            let mut rng = Rng::new(seed);
            let t = ty(&mut rng, 2);
            let [a, b, c] = [value(&mut rng, &t), value(&mut rng, &t), value(&mut rng, &t)];
            let mut cmp = |x: &Value, y: &Value| x.tlc_compare(ev, y).expect("comparable");
            let (ab, ba, bc, ac) = (cmp(&a, &b), cmp(&b, &a), cmp(&b, &c), cmp(&a, &c));
            assert_eq!(cmp(&a, &a), Ordering::Equal, "seed {seed}: reflexive");
            assert_eq!(ab, ba.reverse(), "seed {seed}: antisymmetric");
            if ab == bc {
                assert_eq!(ac, ab, "seed {seed}: transitive");
            }
            assert_eq!(a.tlc_equals(ev, &b).expect("comparable"), ab == Ordering::Equal, "seed {seed}: agrees with equals");
        }
    });
}
