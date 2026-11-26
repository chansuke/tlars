//! `pcal.Validator`: whether a module's PlusCal algorithm and its TLA+ translation still
//! match the checksums the translator wrote into the `\* BEGIN TRANSLATION` line. Written by
//! hand (an inner enum, regexes and `CRC32`); the PlusCal parser and translator it runs to
//! checksum the algorithm are translated.

use super::files::java_lines;
use super::generated::{
    AST, IntPair, PCalTLAGenerator, ParseAlgorithm, ParseUnit, PcalCharReader, PcalDebug, PcalParams, TLAtoPCalMapping, trans,
};
use super::java::*;
use super::support::{ArrayList, JavaVector};

/// `Validator.ValidationResult`.
#[expect(dead_code, reason = "Validator.ValidationResult whole; SANY's validator never returns NoTlaChecksumsExist")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValidationResult {
    NoPlusCalExists,
    NoTranslationExists,
    NoTlaChecksumsExist,
    NoPcalChecksumsExist,
    TlaDivergenceExists,
    PcalDivergenceExists,
    ErrorEncountered,
    NoDivergence,
}

use ValidationResult::*;

const BEGIN_ALG: &str = "--algorithm";
const BEGIN_FAIR_ALG: &str = "--fair";
const FAIR: &str = "fair";

/// `Validator.validate(parseUnit, inputStream)`: the module's lines, between its start and
/// end lines, read as UTF-8.
pub(crate) fn validate(parse_unit: &J<ParseUnit>, source: &[u8]) -> JResult<Vec<ValidationResult>> {
    let loc = parse_unit.getParseTree()?.getLocation()?;
    let (begin, end) = (loc.beginLine(), loc.endLine());
    let text = String::from_utf8_lossy(source);
    let mut lines = Vec::new();
    let mut seen_algo = false;
    // Location is 1-indexed.
    for (cnt, line) in (1..).zip(java_lines(&text)) {
        if begin <= cnt && cnt <= end {
            if line.find(&BEGIN_ALG[2..]).is_some_and(|i| i > 0) {
                seen_algo = true;
            }
            lines.push(line.to_string());
        } else if cnt >= end {
            break;
        }
    }
    if !seen_algo {
        // No "algorithm" string in the input => No PlusCal algorithm.
        return Ok(vec![NoPlusCalExists]);
    }
    validate_lines(&lines)
}

/// `Validator.validate(List<String> specificationText)`.
fn validate_lines(specification_text: &[String]) -> JResult<Vec<ValidationResult>> {
    let input = ArrayList::from_vec(specification_text.iter().map(|l| JStr::of(l)).collect());
    let de_tabbed = trans::removeTabs(input)?;

    // `PlusCal options (...)`.
    let search_loc = IntPair::new_2(0, 0)?;
    loop {
        match find_options(&de_tabbed, &search_loc) {
            Ok(true) => continue,
            Ok(false) => break,
            // The token "PlusCal" not found.
            Err(JThrow::UnrecoverableException(e)) if e.jread(|o| o.is_ParseAlgorithmException()) => break,
            Err(e) => return Err(e),
        }
    }

    // Search for "--algorithm" or "--fair": algLine and algCol end up right after it.
    let mut alg_line = 0;
    let mut alg_col = -1;
    let mut found_begin = false;
    let mut found_fair_begin = false;
    while alg_line < de_tabbed.size() && !found_begin {
        let line: JStr = de_tabbed.elementAt(alg_line);
        let line = line.as_str();
        // `MODULE_CLOSING_PATTERN`, `^[=]+$`.
        if !line.is_empty() && line.chars().all(|c| c == '=') {
            break;
        }
        if let Some(col) = utf16_index_of(line, BEGIN_ALG) {
            alg_col = col + BEGIN_ALG.len() as i32;
            found_begin = true;
        } else if let Some(col) = utf16_index_of(line, BEGIN_FAIR_ALG) {
            alg_col = col + BEGIN_FAIR_ALG.len() as i32;
            found_begin = true;
            found_fair_begin = true;
        } else {
            alg_line += 1;
        }
    }

    let mut res = Vec::new();
    let add = |res: &mut Vec<ValidationResult>, r| {
        if !res.contains(&r) {
            res.push(r);
        }
    };

    // TLA+ translation
    let translation_line = trans::findTokenPair(de_tabbed.clone(), 0, JStr::of("BEGIN"), JStr::of("TRANSLATION"))?;
    if translation_line == -1 {
        add(&mut res, NoTranslationExists);
    }
    let end_translation_line = trans::findTokenPair(de_tabbed.clone(), translation_line + 1, JStr::of("END"), JStr::of("TRANSLATION"))?;
    if end_translation_line == -1 {
        add(&mut res, NoTranslationExists);
    }
    if translation_line == -1 && end_translation_line == -1 {
        return Ok(res);
    }

    // PlusCal algorithm
    if !found_begin {
        add(&mut res, NoPlusCalExists);
    } else {
        // Get the PlusCal AST by parsing it.
        if let Err(e) = ParseAlgorithm::uncomment(de_tabbed.clone(), alg_line, alg_col) {
            report_unrecoverable(e)?;
            return Ok(vec![ErrorEncountered]);
        }
        let mapping = TLAtoPCalMapping::new_0()?;
        mapping.jget_mut().algColumn = alg_col;
        mapping.jget_mut().algLine = alg_line;
        PcalParams::statics().jget_mut().tlaPcalMapping = mapping;

        let mut ast = AST::new_0()?;
        let parsed = (|| -> JResult<()> {
            let reader = PcalCharReader::new_5(de_tabbed.clone(), alg_line, alg_col, specification_text.len() as i32, 0)?;
            ast = ParseAlgorithm::getAlgorithm(reader, found_fair_begin)?;
            // The call translate mutates the ast in pcal.PcalTranslate.Explode(AST, PcalSymTab).
            let generator = PCalTLAGenerator::new_1(ast.clone())?;
            generator.removeNameConflicts()?;
            generator.translate()?;
            Ok(())
        })();
        if let Err(e) = parsed {
            // `catch (ParseAlgorithmException | RemoveNameConflictsException e)`.
            match &e {
                JThrow::UnrecoverableException(x) if x.jread(|o| o.is_ParseAlgorithmException() || o.is_RemoveNameConflictsException()) => {
                    report_unrecoverable(e)?;
                }
                _ => return Err(e),
            }
        }

        let checksum_line: JStr = vector_get(&de_tabbed, translation_line)?;
        match match_checksum_line(checksum_line.as_str()) {
            Some((pcal, tla)) => {
                if let Some(expected) = pcal {
                    // `foundFairBegin ? FAIR : "" + ast.toString()`: a fair algorithm's
                    // checksum is that of "fair".
                    let text = if found_fair_begin { FAIR.to_string() } else { ast.java_string() };
                    if expected != checksum(&text) {
                        // Mismatch between the PlusCal algorithm and its checksum.
                        add(&mut res, PcalDivergenceExists);
                    }
                }
                if let Some(expected) = tla {
                    let from = usize::try_from(translation_line + 1).expect("found line");
                    let to = usize::try_from(end_translation_line).map_err(|_| sublist_error(from as i32, end_translation_line))?;
                    if from > to || to > specification_text.len() {
                        return Err(sublist_error(from as i32, end_translation_line));
                    }
                    let translation: String = specification_text[from..to].concat();
                    if expected != checksum(&translation) {
                        // Mismatch between the TLA+ translation and its checksum.
                        add(&mut res, TlaDivergenceExists);
                    }
                }
            }
            None => add(&mut res, NoPcalChecksumsExist),
        }
    }
    Ok(if res.is_empty() { vec![NoDivergence] } else { res })
}

/// One iteration of the `PlusCal options` search: `Ok(true)` to keep searching.
fn find_options(de_tabbed: &J<JavaVector>, search_loc: &J<IntPair>) -> JResult<bool> {
    ParseAlgorithm::FindToken(JStr::of("PlusCal"), de_tabbed.clone(), search_loc.clone(), JStr::of(""))?;
    let line = ParseAlgorithm::GotoNextNonSpace(de_tabbed.clone(), search_loc.clone())?;
    let rest_of_line = line.substring_from(search_loc.jread(|o| o.two));
    if rest_of_line.starts_with(&JStr::of("options")) && ParseAlgorithm::NextNonIdChar(rest_of_line, 0)? == 7 {
        // The "options" should begin an options line
        PcalParams::statics().jget_mut().optionsInFile = true;
        ParseAlgorithm::ProcessOptions(de_tabbed.clone(), search_loc.clone())?;
        return Ok(false);
    }
    Ok(true)
}

/// `PcalDebug.reportError(e)` for the exceptions the validator catches.
fn report_unrecoverable(e: JThrow) -> JResult<()> {
    match e {
        JThrow::UnrecoverableException(x) => PcalDebug::reportError_1_0(x),
        other => Err(other),
    }
}

/// `String.indexOf` in UTF-16 units.
fn utf16_index_of(line: &str, needle: &str) -> Option<i32> {
    line.find(needle).map(|b| line[..b].encode_utf16().count() as i32)
}

/// `Vector.get(i)`: a negative index fails inside the backing array (its capacity, 10
/// doubled until it holds the elements, as `removeTabs` fills a `new Vector<>()`).
fn vector_get(v: &J<JavaVector>, i: i32) -> JResult<JStr> {
    let n = v.size();
    if i < 0 {
        let mut capacity = 10;
        while capacity < n {
            capacity *= 2;
        }
        return Err(JThrow::new_extern(
            "ArrayIndexOutOfBoundsException",
            JStr::of(&format!("Index {i} out of bounds for length {capacity}")),
        ));
    }
    if i >= n {
        return Err(JThrow::new_extern("ArrayIndexOutOfBoundsException", JStr::of(&format!("Array index out of range: {i}"))));
    }
    Ok(v.elementAt(i))
}

/// `List.subList(from, to)` out of range.
fn sublist_error(from: i32, to: i32) -> JThrow {
    let message = if from > to { format!("fromIndex({from}) > toIndex({to})") } else { format!("toIndex = {to}") };
    JThrow::new_extern(if from > to { "IllegalArgumentException" } else { "IndexOutOfBoundsException" }, JStr::of(&message))
}

/// `Validator.checksum(string)`: CRC-32 of the UTF-8 bytes, as `Long.toHexString`.
pub(crate) fn checksum(text: &str) -> String {
    format!("{:x}", tlars_java::crc32(text.as_bytes()))
}

/// `Validator.CHECKSUM_PATTERN.matcher(line).find()`, with its `pcalchecksum` and
/// `tlachecksum` groups (`None` where the line says `\in STRING`):
///
/// ```text
/// \\\* BEGIN TRANSLATION\s+\(\s*((?i)ch(ec)?ksum\(p(lus)?cal\)(?-i))\s*(=\s*\"(?<pcalchecksum>[0-9A-Fa-f]*)\"|\\in\s*STRING)
///   \s*\/\\\s*((?i)ch(ec)?ksum\(tla\+?\)(?-i))\s*(=\s*\"(?<tlachecksum>[0-9A-Fa-f]*)\"|\\in\s*STRING)\s*\)
/// ```
///
/// Every step is determined by the next character, so a leftmost match is found by trying
/// each occurrence of the leading literal.
pub(crate) fn match_checksum_line(line: &str) -> Option<(Option<String>, Option<String>)> {
    const LEAD: &str = "\\* BEGIN TRANSLATION";
    let mut from = 0;
    while let Some(i) = line[from..].find(LEAD) {
        let start = from + i;
        if let Some(groups) = match_checksums_at(&line[start + LEAD.len()..]) {
            return Some(groups);
        }
        from = start + 1;
    }
    None
}

struct Cursor<'a>(&'a str);

impl Cursor<'_> {
    /// Java's `\s`: space, `\t`, `\n`, `\x0B`, `\f`, `\r`.
    fn spaces(&mut self) -> usize {
        let n = self.0.len() - self.0.trim_start_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r']).len();
        self.0 = &self.0[n..];
        n
    }
    fn lit(&mut self, s: &str) -> bool {
        match self.0.strip_prefix(s) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }
    /// A literal under `(?i)`: ASCII case-insensitive.
    fn lit_ci(&mut self, s: &str) -> bool {
        match self.0.get(..s.len()) {
            Some(head) if head.eq_ignore_ascii_case(s) => {
                self.0 = &self.0[s.len()..];
                true
            }
            _ => false,
        }
    }
    /// `ch(ec)?ksum\(` under `(?i)`.
    fn checksum_word(&mut self) -> bool {
        if !self.lit_ci("ch") {
            return false;
        }
        let save = self.0;
        if self.lit_ci("ec") && self.lit_ci("ksum(") {
            return true;
        }
        self.0 = save;
        self.lit_ci("ksum(")
    }
    /// `(=\s*\"(?<g>[0-9A-Fa-f]*)\"|\\in\s*STRING)`.
    fn value(&mut self) -> Option<Option<String>> {
        if self.lit("=") {
            self.spaces();
            if !self.lit("\"") {
                return None;
            }
            let hex: String = self.0.chars().take_while(char::is_ascii_hexdigit).collect();
            self.0 = &self.0[hex.len()..];
            return self.lit("\"").then_some(Some(hex));
        }
        if self.lit("\\in") {
            self.spaces();
            return self.lit("STRING").then_some(None);
        }
        None
    }
}

fn match_checksums_at(rest: &str) -> Option<(Option<String>, Option<String>)> {
    let mut c = Cursor(rest);
    if c.spaces() == 0 || !c.lit("(") {
        return None;
    }
    c.spaces();
    // `(?i)ch(ec)?ksum\(p(lus)?cal\)`
    if !c.checksum_word() || !c.lit_ci("p") {
        return None;
    }
    let save = c.0;
    if !(c.lit_ci("lus") && c.lit_ci("cal)")) {
        c.0 = save;
        if !c.lit_ci("cal)") {
            return None;
        }
    }
    c.spaces();
    let pcal = c.value()?;
    c.spaces();
    if !c.lit("/\\") {
        return None;
    }
    c.spaces();
    // `(?i)ch(ec)?ksum\(tla\+?\)`
    if !c.checksum_word() || !c.lit_ci("tla") {
        return None;
    }
    c.lit("+");
    if !c.lit(")") {
        return None;
    }
    c.spaces();
    let tla = c.value()?;
    c.spaces();
    c.lit(")").then_some((pcal, tla))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_java() {
        // java.util.zip.CRC32 of "123456789" is cbf43926; of "" is 0.
        assert_eq!(checksum("123456789"), "cbf43926");
        assert_eq!(checksum(""), "0");
    }

    #[test]
    fn checksum_line_forms() {
        let line = r#"\* BEGIN TRANSLATION (chksum(pcal) = "a1b2" /\ chksum(tla) = "ff00")"#;
        assert_eq!(match_checksum_line(line), Some((Some("a1b2".into()), Some("ff00".into()))));
        let line = r#"\* BEGIN TRANSLATION (CheckSum(PlusCal) \in STRING /\ chksum(tla+) = "")"#;
        assert_eq!(match_checksum_line(line), Some((None, Some(String::new()))));
        assert_eq!(match_checksum_line(r"\* BEGIN TRANSLATION"), None);
        assert_eq!(match_checksum_line(r#"\* BEGIN TRANSLATION(chksum(pcal) = "1" /\ chksum(tla) = "2")"#), None);
    }
}
