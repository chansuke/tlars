//! Compares every code's rendered message with TLC's `MP.getMessage`, recorded by
//! `tools/golden/MpGolden.java` (tool and non-tool mode, every class, several parameter
//! counts, `-Dtlc2.output.MP.noTimestamps=true`).

use tlars_msg::{Ec, MsgClass, RenderCx, frame, render};

const CLASSES: [MsgClass; 5] = [MsgClass::None, MsgClass::Error, MsgClass::TlcBug, MsgClass::Warning, MsgClass::State];

#[test]
fn every_message_matches_tlc() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/mp.txt");
    let text = std::fs::read_to_string(path).expect("golden file");
    let mut failures = Vec::new();
    let mut checked = 0;
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(5, '\t').collect();
        let code: i32 = f[0].parse().unwrap();
        let ec = Ec::from_code(code).unwrap_or_else(|| panic!("code {code} missing from the catalog"));
        let class = CLASSES[f[1].parse::<usize>().unwrap()];
        let tool_mode = f[2] == "1";
        let params: Vec<String> = (1..=f[3].parse::<usize>().unwrap()).map(|i| format!("P{i}")).collect();
        let params: Vec<&str> = params.iter().map(String::as_str).collect();
        let expected = f[4].replace("\\n", "\n").replace("\\\\", "\\");

        let cx = RenderCx { class, tool_mode, now: "NOW".into(), debug: false };
        let mut actual = frame(class, ec, &render(ec, &params, &cx), tool_mode);
        if class == MsgClass::Warning && !tool_mode {
            actual.push_str("\n(Use the -nowarning option to disable this warning.)");
        }
        if actual != expected {
            failures.push(format!("{} class={} tool={tool_mode} params={}:\n  tlars: {actual:?}\n  tlc:   {expected:?}", ec.name(), f[1], params.len()));
        }
        checked += 1;
    }
    assert!(checked > 10_000);
    assert!(failures.is_empty(), "{} mismatches, first:\n{}", failures.len(), failures[..failures.len().min(10)].join("\n"));
}

#[test]
fn every_code_is_unique_and_round_trips() {
    for &ec in Ec::ALL {
        assert_eq!(Ec::from_code(ec.code()), Some(ec), "{}", ec.name());
    }
}
