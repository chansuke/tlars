//! `tlc2.pprint.{PrettyPrint, Parse, Format}`: re-parse a value's flat `toString`; if the
//! parse does not consume everything, print it flat; if it fits in `width`, print flat;
//! otherwise break with Format's fixed pads (`{ `, ` }`, `<< `, ` >>`, ` @@`, ` :>`, ` |->`,
//! `SUBSET `). The width does not count the `/\ x = ` prefix.
//!
//! An index here is a byte offset into the flat string and a span's `last` is the offset of
//! its final byte, which keeps `Parse`'s index arithmetic unchanged: every token it steps over
//! is ASCII, and `last--` after a scan still lands on the end of the last character. A node's
//! `length()` is its UTF-16 length, because Java measures a `String` in code units.

/// `PrettyPrint.mypp` (`PrettyPrint.java:15-28`). `PrettyPrint.pp`, which reports a parse or
/// format failure as a TLC bug instead of swallowing it, is not ported: nothing but its own
/// `main` calls it.
pub fn pretty_print(flat: &str, width: usize) -> String {
    let Ok(tree) = parse(flat, 0) else { return flat.to_string() };
    if tree.span().last + 1 < flat.len() {
        return flat.to_string();
    }
    format(flat, &tree, width as i64, 0, "").unwrap_or_else(|_| flat.to_string())
}

/// A `Node`'s extent in the flat string (`Node.java:106-111`).
#[derive(Clone, Copy)]
struct Span {
    first: usize,
    last: usize,
}

impl Span {
    /// `Node.value()`, which throws when the span runs past the end of the string.
    fn value<'a>(&self, flat: &'a str) -> Caught<&'a str> {
        flat.get(self.first..self.last + 1).ok_or(Swallowed)
    }

    /// `Node.length()`, a count of UTF-16 code units.
    fn length(&self, flat: &str) -> i64 {
        match self.value(flat) {
            Ok(text) => units(text),
            // Java reads no characters to measure a node, so a span past the end of the string
            // still has a length and only `value()` throws on it. That happens for a sequence
            // whose second `>` is missing, where the one position past the end stands for a
            // character that would have been one code unit wide.
            Err(Swallowed) => {
                let seen = flat.get(self.first..).map_or(0, units);
                seen + (self.last + 1 - flat.len().max(self.first)) as i64
            }
        }
    }
}

/// The length Java measures a `String` by.
fn units(text: &str) -> i64 {
    text.chars().map(|c| c.len_utf16() as i64).sum()
}

/// A parse tree node. `Node.RECORDPAIR` and `Node.FUNCTIONPAIR` are a separate type because
/// they only ever appear as the children of a record or a function, and so never reach the
/// switch in `Format.format` (`Node.java:10-17`).
enum Node {
    Constant(Span),
    List { span: Span, kind: ListKind, items: Vec<Node> },
    Pairs { span: Span, kind: PairsKind, pairs: Vec<Pair> },
}

/// `Node.SET`, `Node.SEQUENCE`, `Node.SUBSET`: a list of values between delimiters.
#[derive(Clone, Copy)]
enum ListKind {
    Set,
    Sequence,
    Subset,
}

/// `Node.RECORD`, `Node.FUNCTION`: a list of field/value or argument/value pairs.
#[derive(Clone, Copy)]
enum PairsKind {
    Record,
    Function,
}

struct Pair {
    arg: Node,
    val: Node,
}

impl Node {
    fn span(&self) -> Span {
        match self {
            Node::Constant(span) | Node::List { span, .. } | Node::Pairs { span, .. } => *span,
        }
    }
}

/// The `open` and `close` delimiters, the `pad` indenting lines two and higher, and the `sep`
/// between components (`Format.java:45-72`).
struct ListStyle {
    open: &'static str,
    close: &'static str,
    pad: &'static str,
    sep: &'static str,
}

/// The same, plus the `div` between the halves of a pair and the `divpad` indenting a second
/// half that did not fit on the first half's line.
struct PairsStyle {
    open: &'static str,
    close: &'static str,
    pad: &'static str,
    sep: &'static str,
    div: &'static str,
    divpad: &'static str,
}

impl ListKind {
    fn style(self) -> ListStyle {
        match self {
            ListKind::Set => ListStyle { open: "{ ", close: " }", pad: "  ", sep: "," },
            ListKind::Sequence => ListStyle { open: "<< ", close: " >>", pad: "   ", sep: "," },
            ListKind::Subset => ListStyle { open: "SUBSET ", close: " ", pad: " ", sep: "," },
        }
    }
}

impl PairsKind {
    fn style(self) -> PairsStyle {
        match self {
            PairsKind::Record => PairsStyle { open: "[ ", close: " ]", pad: "  ", sep: ",", div: " |->", divpad: "    " },
            PairsKind::Function => PairsStyle { open: "( ", close: " )", pad: "  ", sep: " @@", div: " :>", divpad: "    " },
        }
    }
}

/// `Format.format` (`Format.java:88-132`). A width is signed because the pads subtracted on the
/// way down can take it past zero.
fn format(flat: &str, value: &Node, columnwidth: i64, trailerwidth: i64, leftpad: &str) -> Caught<String> {
    let span = value.span();
    if span.length(flat) <= columnwidth - trailerwidth {
        return Ok(span.value(flat)?.to_string());
    }
    Ok(match value {
        Node::Constant(_) => span.value(flat)?.to_string(),
        // `Format.formatSimpleValue` (`Format.java:144-165`).
        Node::List { kind, items, .. } => {
            let s = kind.style();
            let inner = format_values(
                flat,
                items,
                columnwidth - s.open.len().max(s.pad.len()) as i64,
                trailerwidth + s.close.len() as i64,
                &format!("{leftpad}{}", s.pad),
                s.sep,
            )?;
            format!("{}{inner}{}", s.open, s.close)
        }
        // `Format.formatPairValue` (`Format.java:206-231`).
        Node::Pairs { kind, pairs, .. } => {
            let s = kind.style();
            let inner = format_pairs(
                flat,
                pairs,
                columnwidth - s.open.len().max(s.pad.len()) as i64,
                trailerwidth + s.close.len() as i64,
                &format!("{leftpad}{}", s.pad),
                &s,
            )?;
            format!("{}{inner}{}", s.open, s.close)
        }
    })
}

/// `Format.formatValues` (`Format.java:169-193`).
fn format_values(flat: &str, values: &[Node], columnwidth: i64, trailerwidth: i64, leftpad: &str, sep: &str) -> Caught<String> {
    let mut pp = String::new();
    for (i, value) in values.iter().enumerate() {
        if i + 1 < values.len() {
            // this is not the last value in the list, so leave room for sep
            pp.push_str(&format(flat, value, columnwidth, sep.len() as i64, leftpad)?);
            pp.push_str(sep);
            pp.push('\n');
            pp.push_str(leftpad);
        } else {
            pp.push_str(&format(flat, value, columnwidth, trailerwidth, leftpad)?);
        }
    }
    Ok(pp)
}

/// `Format.formatPairs` (`Format.java:235-324`). The last pair keeps `trailerwidth` free for
/// the closing delimiter, while the others spend it on the value and append `sep` instead.
fn format_pairs(flat: &str, pairs: &[Pair], columnwidth: i64, trailerwidth: i64, leftpad: &str, s: &PairsStyle) -> Caught<String> {
    let (div, divpad, sep) = (s.div.len() as i64, s.divpad.len() as i64, s.sep.len() as i64);
    let mut pp = String::new();
    for (i, pair) in pairs.iter().enumerate() {
        let (arg, val) = (pair.arg.span(), pair.val.span());
        let is_last = i + 1 == pairs.len();
        let (val_trailer, suffix) = match is_last {
            true => (trailerwidth, String::new()),
            false => (sep, format!("{}\n{leftpad}", s.sep)),
        };
        let fits = match is_last {
            true => arg.length(flat) + div + 1 + val.length(flat) <= columnwidth - trailerwidth,
            false => arg.length(flat) + div + 1 + val.length(flat) + sep <= columnwidth,
        };

        if fits {
            pp.push_str(arg.value(flat)?);
            pp.push_str(s.div);
            pp.push(' ');
            pp.push_str(val.value(flat)?);
            pp.push_str(&suffix);
            continue;
        }

        // TLC's remaining two cases, the argument alone on one line and the argument broken
        // over several, lay out the value identically, so only the argument branches here.
        if arg.length(flat) + div <= columnwidth {
            pp.push_str(arg.value(flat)?);
        } else {
            pp.push_str(&format(flat, &pair.arg, columnwidth, trailerwidth + div, leftpad)?);
        }
        pp.push_str(s.div);
        pp.push('\n');
        pp.push_str(leftpad);
        pp.push_str(s.divpad);
        let pad = format!("{leftpad}{}", s.divpad);
        pp.push_str(&format(flat, &pair.val, columnwidth - divpad, val_trailer, &pad)?);
        pp.push_str(&suffix);
    }
    Ok(pp)
}

/// A failure `mypp` swallows: `Parse`'s own `ParseException`, or the
/// `StringIndexOutOfBoundsException` that reading a span past the end of the string throws
/// while formatting. It carries no message, since `mypp` discards the "TLC Bug" texts
/// `Parse` builds for `PrettyPrint.pp`.
struct Swallowed;

type Caught<T> = Result<T, Swallowed>;

/// `String.charAt`, which throws past the end of the string.
fn char_at(flat: &str, index: usize) -> Caught<char> {
    flat.get(index..).and_then(|rest| rest.chars().next()).ok_or(Swallowed)
}

/// `String.regionMatches` and `String.startsWith(s, index)`, both of which answer false rather
/// than throwing when the region runs past the end.
fn region_matches(flat: &str, index: usize, token: &str) -> bool {
    flat.get(index..).is_some_and(|rest| rest.starts_with(token))
}

/// `Character.isWhitespace`, which is Unicode's `White_Space` plus the four information
/// separators, minus the next-line character and the three spaces that do not break a line.
fn is_java_whitespace(c: char) -> bool {
    match c {
        '\u{1c}'..='\u{1f}' => true,
        '\u{85}' | '\u{a0}' | '\u{2007}' | '\u{202f}' => false,
        c => c.is_whitespace(),
    }
}

/// `Character.isDigit`, which is every Unicode decimal digit. TLC writes integers in ASCII,
/// and a non-ASCII digit can only arrive inside a record field name, where it leads to the
/// same span either way: the interval it would let `parseInterval` look for needs a `..`,
/// which `isName` rejects.
fn is_java_digit(c: char) -> bool {
    c.is_ascii_digit()
}

/// `Character.isLetterOrDigit`, for which `char::is_alphanumeric` is the stand-in
/// `FcnRcdValue.isName` already uses on the field names that reach here, and which has to
/// agree with it: a name tlars prints unquoted has to parse back as a constant. The two
/// predicates part company on a letter number, an other number and a combining mark, which
/// Java does not count and which no Unicode category is in `core` to exclude. Java classifies
/// one UTF-16 code unit at a time, so a supplementary character is a surrogate pair and
/// neither half of it is a letter.
fn is_java_letter_or_digit(c: char) -> bool {
    c.len_utf16() == 1 && c.is_alphanumeric()
}

/// `Parse.parse` (`Parse.java:61-103`).
fn parse(flat: &str, index: usize) -> Caught<Node> {
    if flat.is_empty() || index >= flat.len() {
        return Err(Swallowed);
    }
    let index = skip_whitespace(flat, index)?;
    match char_at(flat, index)? {
        '{' => parse_set(flat, index),
        '<' => parse_sequence(flat, index),
        '[' => parse_record(flat, index),
        '(' => parse_function(flat, index),
        _ if region_matches(flat, index, "SUBSET") => parse_subset(flat, index),
        _ => parse_constant(flat, index),
    }
}

/// `Parse.parseSet` (`Parse.java:111-169`).
fn parse_set(flat: &str, index: usize) -> Caught<Node> {
    let (items, last) = parse_list(flat, index + 1, '}', ",", value_element)?;
    Ok(Node::List { span: Span { first: index, last }, kind: ListKind::Set, items })
}

/// `Parse.parseSequence` (`Parse.java:177-237`), whose `last` ends on the second `>` of `>>`.
fn parse_sequence(flat: &str, index: usize) -> Caught<Node> {
    let (items, last) = parse_list(flat, index + 2, '>', ",", value_element)?;
    Ok(Node::List { span: Span { first: index, last: last + 1 }, kind: ListKind::Sequence, items })
}

/// `Parse.parseRecord` (`Parse.java:248-305`).
fn parse_record(flat: &str, index: usize) -> Caught<Node> {
    let (pairs, last) = parse_list(flat, index + 1, ']', ",", |flat, at| parse_pair(flat, at, "|->"))?;
    Ok(Node::Pairs { span: Span { first: index, last }, kind: PairsKind::Record, pairs })
}

/// `Parse.parseFunction` (`Parse.java:373-433`).
fn parse_function(flat: &str, index: usize) -> Caught<Node> {
    let (pairs, last) = parse_list(flat, index + 1, ')', "@@", |flat, at| parse_pair(flat, at, ":>"))?;
    Ok(Node::Pairs { span: Span { first: index, last }, kind: PairsKind::Function, pairs })
}

/// The loop the four container parsers share: elements up to `close`, each followed by
/// whitespace, one optional `sep` and more whitespace. A set, a sequence and a record skip a
/// comma and a function skips `@@`; none of them insists on a separator, and none of them
/// accepts the other's. Returns the elements and the index of `close`.
fn parse_list<T>(
    flat: &str,
    start: usize,
    close: char,
    sep: &str,
    element: impl Fn(&str, usize) -> Caught<(T, usize)>,
) -> Caught<(Vec<T>, usize)> {
    let mut last = skip_whitespace(flat, start)?;
    let mut items = Vec::new();
    while char_at(flat, last)? != close {
        let (item, item_last) = element(flat, last)?;
        items.push(item);
        last = skip_whitespace(flat, item_last + 1)?;
        // Java reads the character before testing it, which cannot run off the end here
        // because the `skipWhitespace` above would already have failed.
        if region_matches(flat, last, sep) {
            last += sep.len();
        }
        last = skip_whitespace(flat, last)?;
    }
    Ok((items, last))
}

/// A set or sequence element, which is a value rather than a pair.
fn value_element(flat: &str, index: usize) -> Caught<(Node, usize)> {
    let node = parse(flat, index)?;
    let last = node.span().last;
    Ok((node, last))
}

/// `Parse.parseRecordPair` (`Parse.java:309-363`) and `Parse.parseFunctionPair`
/// (`Parse.java:472-525`), which differ only in the token between the halves.
fn parse_pair(flat: &str, index: usize, div: &str) -> Caught<(Pair, usize)> {
    let arg = parse(flat, index)?;
    let mut last = skip_whitespace(flat, arg.span().last + 1)?;
    if !region_matches(flat, last, div) {
        return Err(Swallowed);
    }
    last = skip_whitespace(flat, last + div.len())?;
    let val = parse(flat, last)?;
    let last = val.span().last;
    Ok((Pair { arg, val }, last))
}

/// `Parse.parseSubset` (`Parse.java:440-468`).
fn parse_subset(flat: &str, index: usize) -> Caught<Node> {
    let at = skip_whitespace(flat, index + "SUBSET".len())?;
    let elt = parse(flat, at)?;
    let last = elt.span().last;
    Ok(Node::List { span: Span { first: index, last }, kind: ListKind::Subset, items: vec![elt] })
}

/// `Parse.parseConstant` (`Parse.java:533-592`): a string, an interval, a negative integer, or
/// an integer, boolean or model value.
fn parse_constant(flat: &str, index: usize) -> Caught<Node> {
    let first = skip_whitespace(flat, index)?;
    let mut last = first;

    if char_at(flat, first)? == '"' {
        // A string constant ends at the next `"`, so the escaped quote `StringValue` writes as
        // `\"` cuts the constant short and the rest of the value fails to parse.
        loop {
            last += char_at(flat, last)?.len_utf8();
            if char_at(flat, last)? == '"' {
                return Ok(Node::Constant(Span { first, last }));
            }
        }
    }

    if is_java_digit(char_at(flat, last)?) {
        // we might be parsing an constant inteval i..j
        if let Some(node) = parse_interval(flat, index) {
            return Ok(node);
        }
    }

    if char_at(flat, last)? == '-' {
        last += 1;
        while is_java_digit(char_at(flat, last)?) {
            last += 1;
            if last == flat.len() {
                break;
            }
        }
        return Ok(Node::Constant(Span { first, last: last - 1 }));
    }

    if is_java_letter_or_digit(char_at(flat, last)?) {
        while is_java_letter_or_digit(char_at(flat, last)?) {
            last += char_at(flat, last)?.len_utf8();
            if last == flat.len() {
                break;
            }
        }
        return Ok(Node::Constant(Span { first, last: last - 1 }));
    }

    // we have encountered an illegal character in a constant
    Err(Swallowed)
}

/// `Parse.parseInterval` (`Parse.java:602-632`), which answers `None` instead of failing, and
/// starts at the caller's index rather than at the first non-blank character.
fn parse_interval(flat: &str, index: usize) -> Option<Node> {
    let first = index;
    let mut last = first;

    if !is_java_digit(char_at(flat, last).ok()?) {
        return None;
    }
    while last < flat.len() && is_java_digit(char_at(flat, last).ok()?) {
        last += 1;
    }

    if !region_matches(flat, last, "..") {
        return None;
    }
    last += 2;

    if !is_java_digit(char_at(flat, last).ok()?) {
        return None;
    }
    while is_java_digit(char_at(flat, last).ok()?) {
        last += 1;
        if last == flat.len() {
            break;
        }
    }
    Some(Node::Constant(Span { first, last: last - 1 }))
}

/// `Parse.skipWhitespace` (`Parse.java:641-659`).
fn skip_whitespace(flat: &str, index: usize) -> Caught<usize> {
    let mut i = index;
    loop {
        let c = char_at(flat, i)?;
        if !is_java_whitespace(c) {
            return Ok(i);
        }
        i += c.len_utf8();
    }
}
