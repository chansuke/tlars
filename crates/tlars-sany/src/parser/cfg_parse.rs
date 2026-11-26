//! `ModelConfig.parse` (`ModelConfig.java:157-612`) over the translated token manager.

use super::generated::{JThrow, SimpleCharStream, TLAplusParserConstants, TLAplusParserTokenManager, Token};
use super::java::*;
use super::support::{self, Reader, UniqueString};
use crate::cfg::{CfgConstant, CfgError, CfgValue, ModelConfig};
use crate::{Atom, Interner};
use std::sync::Arc;
use tlars_java::{Hashtable, string_hash_code};
use tlars_msg::Ec;

/// The keywords: `configTbl`'s keys. A list ends at a token that is one of them.
const KEYWORDS: &[&str] = &[
    "CONSTANT",
    "CONSTANTS",
    "CONSTRAINT",
    "CONSTRAINTS",
    "ACTION_CONSTRAINT",
    "ACTION_CONSTRAINTS",
    "INVARIANT",
    "INVARIANTS",
    "INIT",
    "NEXT",
    "VIEW",
    "SYMMETRY",
    "SPECIFICATION",
    "PROPERTY",
    "PROPERTIES",
    "ALIAS",
    "POSTCONDITION",
    "POSTCONDITIONS",
    "_PERIODIC",
    "_RL_REWARD",
    "_POSSIBLE",
    "CHECK_DEADLOCK",
];

/// What a keyword fills.
#[derive(Clone, Copy)]
enum Slot {
    /// One name (`INIT Init`); a second one is `CFG_TWICE_KEYWORD`.
    Single(fn(&mut ModelConfig) -> &mut Option<String>),
    /// Names until the next keyword.
    List(fn(&mut ModelConfig) -> &mut Vec<String>),
    Constants,
    CheckDeadlock,
}

fn slot(keyword: &str) -> Option<Slot> {
    Some(match keyword {
        "INIT" => Slot::Single(|c| &mut c.init),
        "NEXT" => Slot::Single(|c| &mut c.next),
        "SPECIFICATION" => Slot::Single(|c| &mut c.specification),
        "VIEW" => Slot::Single(|c| &mut c.view),
        "SYMMETRY" => Slot::Single(|c| &mut c.symmetry),
        "ALIAS" => Slot::Single(|c| &mut c.alias),
        "_PERIODIC" => Slot::Single(|c| &mut c.periodic),
        "_RL_REWARD" => Slot::Single(|c| &mut c.rl_reward),
        "POSTCONDITION" | "POSTCONDITIONS" => Slot::List(|c| &mut c.postconditions),
        "INVARIANT" | "INVARIANTS" => Slot::List(|c| &mut c.invariants),
        "PROPERTY" | "PROPERTIES" => Slot::List(|c| &mut c.properties),
        "CONSTRAINT" | "CONSTRAINTS" => Slot::List(|c| &mut c.constraints),
        "ACTION_CONSTRAINT" | "ACTION_CONSTRAINTS" => Slot::List(|c| &mut c.action_constraints),
        "_POSSIBLE" => Slot::List(|c| &mut c.possible),
        "CONSTANT" | "CONSTANTS" => Slot::Constants,
        "CHECK_DEADLOCK" => Slot::CheckDeadlock,
        _ => return None,
    })
}

fn err(code: Ec, params: &[&str]) -> CfgError {
    CfgError::ConfigFile { code, params: params.iter().map(|p| p.to_string()).collect() }
}

/// The token manager and its char stream, reporting what TLC's helpers report.
struct Lexer {
    tmgr: J<TLAplusParserTokenManager>,
    scs: J<SimpleCharStream>,
}

/// One token: its kind, image and line.
struct Tok {
    kind: i32,
    image: String,
    begin_line: i32,
}

impl Tok {
    fn is_eof(&self) -> bool {
        self.kind == TLAplusParserConstants::EOF
    }
}

impl Lexer {
    /// `getNextToken(tmgr)`: a `TokenMgrError` is `CFG_LEXICAL_ERROR` with its message.
    fn next(&self) -> Result<Tok, CfgError> {
        match self.tmgr.getNextToken() {
            Ok(t) => Ok(t.jread(|t: &Token| Tok { kind: t.kind, image: t.image.java_string(), begin_line: t.beginLine })),
            Err(e @ JThrow::TokenMgrError(_)) => {
                let message = e.getMessage().map(|m| m.java_string()).unwrap_or_default();
                Err(err(Ec::CFG_LEXICAL_ERROR, &[&message]))
            }
            Err(other) => panic!("the token manager threw {other:?}"),
        }
    }

    /// `getNextToken(tmgr, buf)`: also appends the image and a space to the raw constants.
    fn next_into(&self, buf: &mut String) -> Result<Tok, CfgError> {
        let t = self.next()?;
        buf.push_str(&t.image);
        buf.push(' ');
        Ok(t)
    }

    /// `scs.getBeginLine()`: the line of the token just read.
    fn line(&self) -> String {
        self.scs.getBeginLine().expect("getBeginLine").to_string()
    }
}

/// `ModelValue.make` and `new StringValue`: intern, and record model values once each.
struct Values<'a> {
    model_values: &'a mut Vec<Atom>,
}

impl Values<'_> {
    fn string(&self, s: &str) -> Atom {
        UniqueString::uniqueStringOf(JStr::of(s)).as_atom().expect("interned")
    }

    fn model_value(&mut self, s: &str) -> Atom {
        let a = self.string(s);
        if !self.model_values.contains(&a) {
            self.model_values.push(a);
        }
        a
    }
}

/// `parseValue`.
fn parse_value(t: Tok, lx: &Lexer, buf: &mut String, values: &mut Values<'_>) -> Result<CfgValue, CfgError> {
    if t.kind == TLAplusParserConstants::NUMBER_LITERAL {
        return t.image.parse::<i32>().map(CfgValue::Int).map_err(|_| CfgError::JavaNumberFormat { image: t.image });
    }
    if t.kind == TLAplusParserConstants::STRING_LITERAL {
        let contents = &t.image[1..t.image.len() - 1];
        if contents.contains('\\') {
            let message = format!("escape sequences are not supported in strings at line {}", t.begin_line);
            return Err(err(Ec::CFG_LEXICAL_ERROR, &[&message]));
        }
        return Ok(CfgValue::Str(values.string(contents)));
    }
    match t.image.as_str() {
        "TRUE" => return Ok(CfgValue::Bool(true)),
        "FALSE" => return Ok(CfgValue::Bool(false)),
        "{" => {
            let mut elems = Vec::new();
            let mut t = lx.next_into(buf)?;
            if t.image != "}" {
                loop {
                    elems.push(parse_value(t, lx, buf, values)?);
                    t = lx.next_into(buf)?;
                    if t.image != "," {
                        break;
                    }
                    t = lx.next_into(buf)?;
                }
            }
            if t.image != "}" {
                return Err(err(Ec::CFG_EXPECTED_SYMBOL, &[&lx.line(), "}"]));
            }
            return Ok(CfgValue::Set(elems));
        }
        _ => {}
    }
    if !t.is_eof() {
        return Ok(CfgValue::ModelValue(values.model_value(&t.image)));
    }
    Err(err(Ec::CFG_EXPECTED_SYMBOL, &[&lx.line(), "a value"]))
}

fn hash(s: &str) -> i32 {
    string_hash_code(s)
}

/// One `CONSTANT` section, from the token after the keyword; returns the token that ends it.
fn parse_constants(lx: &Lexer, cfg: &mut ModelConfig, keyword: &str, loc: i32) -> Result<Tok, CfgError> {
    let mut buf = keyword.to_string();
    let mut model_values = std::mem::take(&mut cfg.model_values);
    let result = (|| -> Result<Tok, CfgError> {
        let mut values = Values { model_values: &mut model_values };
        loop {
            let t = lx.next()?;
            if t.is_eof() || KEYWORDS.contains(&t.image.as_str()) {
                return Ok(t);
            }
            buf.push('\n');
            buf.push_str(&t.image);
            buf.push(' ');
            let mut lhs = t.image;
            let mut t = lx.next_into(&mut buf)?;
            while t.image == "!" {
                t = lx.next_into(&mut buf)?;
                lhs = format!("{lhs}!{}", t.image);
                t = lx.next_into(&mut buf)?;
            }
            if t.image == "<-" {
                t = lx.next_into(&mut buf)?;
                if t.image == "[" {
                    t = lx.next_into(&mut buf)?;
                    if t.is_eof() {
                        return Err(err(Ec::CFG_EXPECT_ID, &[&lx.line(), "<-["]));
                    }
                    let module = t.image;
                    t = lx.next_into(&mut buf)?;
                    if t.image != "]" {
                        return Err(err(Ec::CFG_EXPECTED_SYMBOL, &[&lx.line(), "]"]));
                    }
                    t = lx.next_into(&mut buf)?;
                    if t.is_eof() {
                        return Err(err(Ec::CFG_EXPECT_ID, &[&lx.line(), "<-[mod]"]));
                    }
                    let h = hash(&module);
                    if cfg.mod_overrides.get(&module, h).is_none() {
                        cfg.mod_overrides.put(module.clone(), h, Hashtable::new());
                    }
                    cfg.mod_overrides.get_mut(&module, h).expect("just added").put(lhs.clone(), hash(&lhs), t.image);
                } else {
                    if t.is_eof() {
                        return Err(err(Ec::CFG_EXPECT_ID, &[&lx.line(), "<-"]));
                    }
                    cfg.overrides.put(lhs.clone(), hash(&lhs), t.image.clone());
                    cfg.overrides_reverse.put(t.image.clone(), hash(&t.image), lhs);
                }
                continue;
            }
            let mut args = Vec::new();
            if t.image == "(" {
                loop {
                    t = lx.next_into(&mut buf)?;
                    args.push(parse_value(t, lx, &mut buf, &mut values)?);
                    t = lx.next_into(&mut buf)?;
                    if t.image != "," {
                        break;
                    }
                }
                if t.image != ")" {
                    return Err(err(Ec::CFG_GENERAL, &[&loc.to_string()]));
                }
                t = lx.next_into(&mut buf)?;
            }
            if t.image != "=" {
                return Err(err(Ec::CFG_EXPECTED_SYMBOL, &[&lx.line(), "= or <-"]));
            }
            t = lx.next_into(&mut buf)?;
            if t.image == "[" {
                t = lx.next_into(&mut buf)?;
                if t.is_eof() {
                    return Err(err(Ec::CFG_EXPECT_ID, &[&lx.line(), "=["]));
                }
                let module = t.image;
                t = lx.next_into(&mut buf)?;
                if t.image != "]" {
                    return Err(err(Ec::CFG_EXPECTED_SYMBOL, &[&lx.line(), "]"]));
                }
                t = lx.next_into(&mut buf)?;
                let value = parse_value(t, lx, &mut buf, &mut values)?;
                let h = hash(&module);
                if cfg.mod_constants.get(&module, h).is_none() {
                    cfg.mod_constants.put(module.clone(), h, Vec::new());
                }
                cfg.mod_constants.get_mut(&module, h).expect("just added").push(CfgConstant { lhs, args, value });
            } else {
                let value = parse_value(t, lx, &mut buf, &mut values)?;
                cfg.constants.push(CfgConstant { lhs, args, value });
            }
        }
    })();
    cfg.model_values = model_values;
    cfg.raw_constants.push(buf);
    result
}

/// `ModelConfig.parse` over `text` (TLC appends a newline).
pub(crate) fn parse_cfg(text: &[u8], interner: &Arc<Interner>) -> Result<ModelConfig, CfgError> {
    support::enter(interner);
    let mut bytes = text.to_vec();
    bytes.push(b'\n');
    let scs = SimpleCharStream::new_3(Reader::utf8(&bytes), 1, 1).expect("char stream");
    let tmgr = TLAplusParserTokenManager::new_2(scs.clone(), 2).expect("token manager");
    let lx = Lexer { tmgr, scs };
    let mut cfg = ModelConfig::default();
    let mut t = lx.next()?;
    while !t.is_eof() {
        let loc = lx.scs.getBeginLine().expect("getBeginLine");
        let keyword = t.image.clone();
        let line = loc.to_string();
        t = match slot(&keyword) {
            Some(Slot::Single(field)) => {
                let name = lx.next()?;
                if name.is_eof() {
                    return Err(err(Ec::CFG_MISSING_ID, &[&line, &keyword]));
                }
                let target = field(&mut cfg);
                if target.is_some() {
                    return Err(err(Ec::CFG_TWICE_KEYWORD, &[&line, &keyword]));
                }
                *target = Some(name.image);
                lx.next()?
            }
            Some(Slot::List(field)) => loop {
                let item = lx.next()?;
                if item.is_eof() || KEYWORDS.contains(&item.image.as_str()) {
                    break item;
                }
                field(&mut cfg).push(item.image);
            },
            Some(Slot::Constants) => parse_constants(&lx, &mut cfg, &keyword, loc)?,
            Some(Slot::CheckDeadlock) => {
                let value = lx.next()?;
                if value.is_eof() {
                    return Err(err(Ec::CFG_MISSING_ID, &[&line, &keyword]));
                }
                let v = match value.image.as_str() {
                    "TRUE" => true,
                    "FALSE" => false,
                    _ => return Err(err(Ec::CFG_EXPECTED_SYMBOL, &[&lx.line(), "TRUE or FALSE"])),
                };
                if cfg.check_deadlock.is_some() {
                    return Err(err(Ec::CFG_TWICE_KEYWORD, &[&line, &keyword]));
                }
                cfg.check_deadlock = Some(v);
                lx.next()?
            }
            None => return Err(err(Ec::CFG_EXPECTED_SYMBOL, &[&lx.line(), "a keyword"])),
        };
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Result<ModelConfig, CfgError>, Arc<Interner>) {
        let interner = Arc::new(Interner::new());
        (parse_cfg(text.as_bytes(), &interner), interner)
    }

    #[test]
    fn keywords_lists_and_constants() {
        let (cfg, interner) = parse("INIT Init\nNEXT Next \\* comment\nINVARIANTS A B\nCONSTANT N = 3\n  S = {a, \"x\", TRUE}\n  F <- G\n");
        let cfg = cfg.expect("parses");
        assert_eq!(cfg.init.as_deref(), Some("Init"));
        assert_eq!(cfg.next.as_deref(), Some("Next"));
        assert_eq!(cfg.invariants, ["A", "B"]);
        assert_eq!(cfg.constants.len(), 2);
        assert_eq!(cfg.constants[0].value, CfgValue::Int(3));
        let CfgValue::Set(items) = &cfg.constants[1].value else { panic!("set") };
        assert_eq!(items.len(), 3);
        assert_eq!(cfg.model_values.len(), 1);
        assert_eq!(&*interner.text(cfg.model_values[0]), "a");
        assert_eq!(cfg.overrides.get(&"F".to_string(), string_hash_code("F")).map(String::as_str), Some("G"));
    }

    #[test]
    fn errors_carry_tlc_codes() {
        assert_eq!(parse("INIT A\nINIT B\n").0.err(), Some(err(Ec::CFG_TWICE_KEYWORD, &["2", "INIT"])));
        assert_eq!(parse("CONSTANT N 3\n").0.err(), Some(err(Ec::CFG_EXPECTED_SYMBOL, &["1", "= or <-"])));
        assert_eq!(parse("FOO\n").0.err(), Some(err(Ec::CFG_EXPECTED_SYMBOL, &["1", "a keyword"])));
        assert!(matches!(parse("CONSTANT N = 99999999999\n").0, Err(CfgError::JavaNumberFormat { .. })));
    }
}
