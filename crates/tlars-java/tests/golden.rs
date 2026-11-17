//! Compares tlars-java with JDK 21 output recorded by `tools/golden/JavaGolden.java`.

use tlars_java::time::Pattern;
use tlars_java::{Hashtable, JavaRandom, decimal_format_grouped, double_to_string, reciprocal_two_digits, string_hash_code};

fn golden(name: &str) -> String {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn string_hash_code_matches_java() {
    for line in golden("string_hash.txt").lines() {
        let (hash, cps) = line.split_once('\t').unwrap();
        let s: String = cps
            .split(',')
            .filter(|c| !c.is_empty())
            .map(|c| char::from_u32(c.parse().unwrap()).unwrap())
            .collect();
        assert_eq!(string_hash_code(&s), hash.parse::<i32>().unwrap(), "{s:?}");
    }
}

#[test]
fn hashtable_enumeration_matches_java() {
    let text = golden("hashtable.txt");
    let mut table = Hashtable::new();
    let mut cases = 0;
    for line in text.lines() {
        if line == "case" {
            table = Hashtable::new();
        } else if let Some(key) = line.strip_prefix("put ") {
            table.put(key.to_string(), string_hash_code(key), ());
        } else if let Some(order) = line.strip_prefix("order ") {
            let actual: Vec<&str> = table.elements().map(|(k, _)| k.as_str()).collect();
            let expected: Vec<&str> = order.split(',').filter(|k| !k.is_empty()).collect();
            assert_eq!(actual, expected);
            cases += 1;
        }
    }
    assert_eq!(cases, 60);
}

#[test]
fn random_matches_java() {
    let mut rng = JavaRandom::new(0);
    for line in golden("random.txt").lines() {
        let mut words = line.split(' ');
        match words.next().unwrap() {
            "seed" => rng = JavaRandom::new(words.next().unwrap().parse().unwrap()),
            "reseed" => rng.set_seed(words.next().unwrap().parse().unwrap()),
            "int" => assert_eq!(rng.next_int(), words.next().unwrap().parse::<i32>().unwrap()),
            "bounded" => {
                let bound = words.next().unwrap().parse().unwrap();
                assert_eq!(rng.next_int_bounded(bound), words.next().unwrap().parse::<i32>().unwrap());
            }
            "long" => assert_eq!(rng.next_long(), words.next().unwrap().parse::<i64>().unwrap()),
            "double" => {
                let bits = u64::from_str_radix(words.next().unwrap(), 16).unwrap();
                assert_eq!(rng.next_double().to_bits(), bits);
            }
            other => panic!("unknown line {other}"),
        }
    }
}

#[test]
fn double_to_string_matches_jdk21() {
    let mut failures = Vec::new();
    for line in golden("double_to_string.txt").lines() {
        let (bits, expected) = line.split_once('\t').unwrap();
        let v = f64::from_bits(u64::from_str_radix(bits, 16).unwrap());
        let actual = double_to_string(v);
        if actual != expected {
            failures.push(format!("{bits}: {actual} != {expected}"));
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures[..failures.len().min(20)].join("\n"));
}

#[test]
fn reciprocal_matches_bigdecimal() {
    for line in golden("reciprocal.txt").lines() {
        let (n, bits) = line.split_once('\t').unwrap();
        let n: i64 = n.parse().unwrap();
        assert_eq!(reciprocal_two_digits(n).to_bits(), u64::from_str_radix(bits, 16).unwrap(), "1/{n}");
    }
}

#[test]
fn decimal_format_matches_java() {
    for line in golden("decimal_format.txt").lines() {
        let (n, expected) = line.split_once('\t').unwrap();
        assert_eq!(decimal_format_grouped(n.parse().unwrap()), expected);
    }
}

#[test]
fn date_format_matches_java() {
    let patterns = [
        Pattern::Timestamp,
        Pattern::DayHour,
        Pattern::HourMinute,
        Pattern::MinuteSecond,
        Pattern::Second,
        Pattern::MetaDir,
        Pattern::MetaDirSeconds,
    ];
    for line in golden("date_format.txt").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let pattern = patterns.iter().find(|p| p.java_pattern() == f[0]).unwrap();
        let actual = tlars_java::time::format(*pattern, f[2].parse().unwrap(), f[1].parse().unwrap());
        assert_eq!(actual, f[3], "{line}");
    }
}

#[test]
fn int_hashset_order_matches_java() {
    use tlars_java::JavaIntHashSet;
    let parse = |s: &str| -> Vec<i32> { s.split(',').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect() };
    for line in golden("int_hashset.txt").lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let mut a = JavaIntHashSet::new();
        for v in parse(f[0]) {
            a.add(v);
        }
        let mut b = JavaIntHashSet::new();
        for v in parse(f[1]) {
            b.add(v);
        }
        assert_eq!(a.to_java_string(), f[2], "{line}");
        let mut inter = JavaIntHashSet::copy_of(&a);
        inter.retain(|v| b.contains(v));
        assert_eq!(inter.to_java_string(), f[3], "{line}");
    }
}
