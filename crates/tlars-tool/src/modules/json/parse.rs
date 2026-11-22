//! `com.google.gson.JsonParser.parseReader(Reader)` at gson 2.14.0: the lenient
//! `JsonReader` that `tlc2.module.Json` reads documents with.
//!
//! The reader walks UTF-16 code units, which is what `locationString`'s column counts and
//! what the 1024-unit buffer `peekNumber` gives up on measures, so the port walks them too
//! and turns a token's units into a `String` only where gson builds one.

use super::node::Node;

/// Why a parse produced no value.
pub(super) enum Failure {
    /// The `getMessage()` of the exception gson throws, which is what `MethodValue.eval`
    /// puts into TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE.
    Thrown(String),
    /// A `\\u` escape naming an unpaired surrogate. gson keeps the lone code unit in the
    /// `String` it builds and a Rust `String` cannot hold one, so there is no value to
    /// answer with.
    UnpairedSurrogate,
}

/// `JsonParser.parseReader(Reader)`.
pub(super) fn parse_reader(text: &str) -> Result<Node, Failure> {
    let mut reader = JsonReader::new(text);
    parse_document(&mut reader).map_err(|thrown| thrown.failure())
}

/// The body of `parseReader(Reader)` (`JsonParser.java:107-120`). The trailing-data check
/// is skipped for a `JsonNull`, so a document that starts with `null` may be followed by
/// anything at all; and it runs after the `finally` of `parseReader(JsonReader)` has put
/// LEGACY_STRICT back, which is why trailing data is reported as malformed JSON and not as
/// `"Did not consume the entire document."`.
fn parse_document(reader: &mut JsonReader) -> Result<Node, Thrown> {
    let element = parse_json_reader(reader)?;
    if !matches!(element, Node::Null) && reader.peek()? != Token::EndDocument {
        // Unreachable: under LEGACY_STRICT a second top-level token makes `doPeek`'s
        // NONEMPTY_DOCUMENT branch call `checkLenient`, which throws instead of answering.
        return Err(Thrown::Syntax("Did not consume the entire document.".to_string()));
    }
    Ok(element)
}

/// `parseReader(JsonReader)` (`JsonParser.java:136-150`): LEGACY_STRICT becomes LENIENT for
/// the parse and is restored in a `finally`, even when the parse threw.
fn parse_json_reader(reader: &mut JsonReader) -> Result<Node, Thrown> {
    reader.strictness = Strictness::Lenient;
    let parsed = streams_parse(reader);
    reader.strictness = Strictness::LegacyStrict;
    parsed
}

/// `Streams.parse(JsonReader)` (`Streams.java:43-66`): an EOF on a document that has not
/// produced a token yet is a `JsonNull`, for compatibility with gson 1.5; an EOF after that
/// is a syntax error.
fn streams_parse(reader: &mut JsonReader) -> Result<Node, Thrown> {
    match reader.peek() {
        Err(Thrown::Eof(_)) => return Ok(Node::Null),
        Err(thrown) => return Err(thrown),
        Ok(_) => {}
    }
    read_element(reader)
}

/// `JsonElementTypeAdapter.read` (`JsonElementTypeAdapter.java:76-136`), which walks with an
/// explicit stack, so the only depth gate is `JsonReader.push`'s nesting limit.
///
/// gson puts the empty array or object into its parent before filling it; holding it back
/// until it is finished builds the same tree, because `JsonObject.add` of a name that is
/// already there replaces the value and keeps the name's first position either way.
fn read_element(reader: &mut JsonReader) -> Result<Node, Thrown> {
    let peeked = reader.peek()?;
    let Some(mut current) = try_begin_nesting(reader, peeked)? else {
        return read_terminal(reader, peeked);
    };
    let mut stack: Vec<(Node, Option<String>)> = Vec::new();
    loop {
        while reader.has_next()? {
            let name = if matches!(current, Node::Object(_)) { Some(reader.next_name()?) } else { None };
            let peeked = reader.peek()?;
            match try_begin_nesting(reader, peeked)? {
                Some(child) => {
                    stack.push((current, name));
                    current = child;
                }
                None => {
                    let value = read_terminal(reader, peeked)?;
                    add_member(&mut current, name, value);
                }
            }
        }
        if matches!(current, Node::Array(_)) {
            reader.end_array()?;
        } else {
            reader.end_object()?;
        }
        match stack.pop() {
            None => return Ok(current),
            Some((mut parent, name)) => {
                add_member(&mut parent, name, current);
                current = parent;
            }
        }
    }
}

/// `tryBeginNesting` (`JsonElementTypeAdapter.java:44-55`).
fn try_begin_nesting(reader: &mut JsonReader, peeked: Token) -> Result<Option<Node>, Thrown> {
    match peeked {
        Token::BeginArray => {
            reader.begin_array()?;
            Ok(Some(Node::Array(Vec::new())))
        }
        Token::BeginObject => {
            reader.begin_object()?;
            Ok(Some(Node::Object(Vec::new())))
        }
        _ => Ok(None),
    }
}

/// `readTerminal` (`JsonElementTypeAdapter.java:58-74`).
fn read_terminal(reader: &mut JsonReader, peeked: Token) -> Result<Node, Thrown> {
    match peeked {
        Token::Str => Ok(Node::Str(reader.next_string()?)),
        // `new JsonPrimitive(new LazilyParsedNumber(in.nextString()))`, so what is kept is
        // the literal the document wrote, down to the case of its `E` and a leading `-0`.
        Token::Number => Ok(Node::Number(reader.next_string()?)),
        Token::Boolean => Ok(Node::Bool(reader.next_boolean()?)),
        Token::Null => {
            reader.next_null()?;
            Ok(Node::Null)
        }
        // The `IllegalStateException` of the default branch, and every
        // `unexpectedTokenError` of the reader, need a token that `hasNext` and `peek` have
        // already ruled out at this point.
        other => unreachable!("the tree adapter asks only for the token it peeked, not {other:?}"),
    }
}

/// `JsonArray.add(value)` or `JsonObject.add(name, value)`.
fn add_member(current: &mut Node, name: Option<String>, value: Node) {
    match (current, name) {
        (Node::Array(items), _) => items.push(value),
        (object, Some(name)) => object.add(name, value),
        (other, None) => unreachable!("a member of {other:?} carries a name"),
    }
}

/// What a thrown exception becomes once `JsonParser` and `Streams` have wrapped it.
enum Thrown {
    /// `new JsonSyntaxException(String)`, whose message is that string.
    Syntax(String),
    /// `MalformedJsonException`, which reaches the caller as the `JsonSyntaxException`
    /// built from it.
    Malformed(String),
    /// `EOFException`, likewise.
    Eof(String),
    /// Not gson's: the one value this port cannot build, carried the same way so that any
    /// reader method can give up where gson would have gone on.
    Unpaired,
}

impl Thrown {
    /// `RuntimeException(Throwable)` takes `cause.toString()` as its message, so the text
    /// the module reports starts with the cause's own class name.
    fn failure(&self) -> Failure {
        match self {
            Thrown::Syntax(message) => Failure::Thrown(message.clone()),
            Thrown::Malformed(message) => {
                Failure::Thrown(format!("com.google.gson.stream.MalformedJsonException: {message}"))
            }
            Thrown::Eof(message) => Failure::Thrown(format!("java.io.EOFException: {message}")),
            Thrown::Unpaired => Failure::UnpairedSurrogate,
        }
    }
}

/// `Strictness`. STRICT cannot occur: `parseReader` only ever swaps LEGACY_STRICT for
/// LENIENT, so the reader's STRICT-only refusals (unescaped control characters, `\'`,
/// upper-cased keywords) are not reachable here.
#[derive(Clone, Copy, PartialEq)]
enum Strictness {
    Lenient,
    LegacyStrict,
}

/// `JsonScope` (`JsonScope.java:25-50`). CLOSED cannot occur: nothing closes the reader
/// before the parse ends.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Scope {
    EmptyArray,
    NonemptyArray,
    EmptyObject,
    DanglingName,
    NonemptyObject,
    EmptyDocument,
    NonemptyDocument,
}

/// The `PEEKED_*` constants (`JsonReader.java:214-237`). PEEKED_BUFFERED is left out: only
/// `nextDouble`, `nextLong` and `nextInt` produce it, and the tree adapter calls none of
/// them.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Peeked {
    None,
    BeginObject,
    EndObject,
    BeginArray,
    EndArray,
    True,
    False,
    Null,
    SingleQuoted,
    DoubleQuoted,
    Unquoted,
    SingleQuotedName,
    DoubleQuotedName,
    UnquotedName,
    Long,
    Number,
    Eof,
}

/// `JsonToken`, as `peek()` reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    BeginObject,
    EndObject,
    BeginArray,
    EndArray,
    Name,
    Str,
    Number,
    Boolean,
    Null,
    EndDocument,
}

/// The number state machine's `NUMBER_CHAR_*` constants (`JsonReader.java:240-247`).
#[derive(Clone, Copy, PartialEq)]
enum NumberChar {
    None,
    Sign,
    Digit,
    Decimal,
    FractionDigit,
    ExpE,
    ExpSign,
    ExpDigit,
}

/// `JsonReader.BUFFER_SIZE`. A literal this long or longer makes `peekNumber` give up and
/// become an unquoted string, so the length of gson's buffer is observable.
const BUFFER_SIZE: usize = 1024;

/// `JsonReader.DEFAULT_NESTING_LIMIT`. Nothing calls `setNestingLimit`.
const NESTING_LIMIT: usize = 255;

/// `Long.MIN_VALUE / 10`, the point from which another digit may overflow.
const MIN_INCOMPLETE_INTEGER: i64 = i64::MIN / 10;

/// `")]}'\n"`, the non-execute prefix (`JsonReader.java:1846-1851`).
const NON_EXECUTE_PREFIX: [u16; 5] = [b')' as u16, b']' as u16, b'}' as u16, b'\'' as u16, b'\n' as u16];

/// `com.google.gson.stream.JsonReader`, reading a document that is already in memory: the
/// whole text stands in for the 1024-unit window `fillBuffer` slides over it, and
/// `fillBuffer(n)` becomes "are there `n` units left". The window is only observable
/// through `peekNumber`, which measures against its length.
struct JsonReader {
    buffer: Vec<u16>,
    pos: usize,
    line_number: i64,
    line_start: usize,
    strictness: Strictness,
    peeked: Peeked,
    peeked_long: i64,
    peeked_number_length: usize,
    stack: Vec<Scope>,
    path_names: Vec<Option<String>>,
    path_indices: Vec<i64>,
}

impl JsonReader {
    /// `new JsonReader(Reader)`, plus the byte order mark `fillBuffer` drops on the first
    /// read (`JsonReader.java:1503-1508`). That check needs `lineStart == 0` and
    /// `lineNumber == 0`, which only hold before anything has been consumed, and it moves
    /// `lineStart` along with `pos`, so the mark costs no column.
    fn new(text: &str) -> JsonReader {
        let buffer: Vec<u16> = text.encode_utf16().collect();
        let bom = usize::from(buffer.first() == Some(&0xfeff));
        JsonReader {
            buffer,
            pos: bom,
            line_number: 0,
            line_start: bom,
            strictness: Strictness::LegacyStrict,
            peeked: Peeked::None,
            peeked_long: 0,
            peeked_number_length: 0,
            stack: vec![Scope::EmptyDocument],
            path_names: vec![None],
            path_indices: vec![0],
        }
    }

    /// The unit at `i` as a character. An unpaired surrogate stands in as U+FFFD, which
    /// every comparison the reader makes treats the same way: neither is whitespace, a
    /// structural character nor part of a keyword or number. What a token's text is made of
    /// is read out of [`JsonReader::buffer`], not out of this.
    fn char_at(&self, i: usize) -> char {
        char::from_u32(u32::from(self.buffer[i])).unwrap_or(char::REPLACEMENT_CHARACTER)
    }

    /// `peek()` (`JsonReader.java:542-579`).
    fn peek(&mut self) -> Result<Token, Thrown> {
        let peeked = self.peeked_token()?;
        Ok(match peeked {
            Peeked::BeginObject => Token::BeginObject,
            Peeked::EndObject => Token::EndObject,
            Peeked::BeginArray => Token::BeginArray,
            Peeked::EndArray => Token::EndArray,
            Peeked::SingleQuotedName | Peeked::DoubleQuotedName | Peeked::UnquotedName => Token::Name,
            Peeked::True | Peeked::False => Token::Boolean,
            Peeked::Null => Token::Null,
            Peeked::SingleQuoted | Peeked::DoubleQuoted | Peeked::Unquoted => Token::Str,
            Peeked::Long | Peeked::Number => Token::Number,
            Peeked::Eof => Token::EndDocument,
            Peeked::None => unreachable!("doPeek never leaves PEEKED_NONE behind"),
        })
    }

    /// `int p = peeked; if (p == PEEKED_NONE) { p = doPeek(); }`, which every consuming
    /// method starts with.
    fn peeked_token(&mut self) -> Result<Peeked, Thrown> {
        if self.peeked == Peeked::None {
            return self.do_peek();
        }
        Ok(self.peeked)
    }

    /// `hasNext()` (`JsonReader.java:533-539`).
    fn has_next(&mut self) -> Result<bool, Thrown> {
        let peeked = self.peeked_token()?;
        Ok(peeked != Peeked::EndObject && peeked != Peeked::EndArray && peeked != Peeked::Eof)
    }

    /// `beginArray()` (`JsonReader.java:458-470`).
    fn begin_array(&mut self) -> Result<(), Thrown> {
        let peeked = self.peeked_token()?;
        assert_eq!(peeked, Peeked::BeginArray, "beginArray is called only where peek answered BEGIN_ARRAY");
        self.push(Scope::EmptyArray)?;
        *self.path_indices.last_mut().unwrap() = 0;
        self.peeked = Peeked::None;
        Ok(())
    }

    /// `endArray()` (`JsonReader.java:478-490`).
    fn end_array(&mut self) -> Result<(), Thrown> {
        let peeked = self.peeked_token()?;
        assert_eq!(peeked, Peeked::EndArray, "endArray is called only where hasNext answered false in an array");
        self.pop();
        *self.path_indices.last_mut().unwrap() += 1;
        self.peeked = Peeked::None;
        Ok(())
    }

    /// `beginObject()` (`JsonReader.java:498-509`).
    fn begin_object(&mut self) -> Result<(), Thrown> {
        let peeked = self.peeked_token()?;
        assert_eq!(peeked, Peeked::BeginObject, "beginObject is called only where peek answered BEGIN_OBJECT");
        self.push(Scope::EmptyObject)?;
        self.peeked = Peeked::None;
        Ok(())
    }

    /// `endObject()` (`JsonReader.java:517-530`).
    fn end_object(&mut self) -> Result<(), Thrown> {
        let peeked = self.peeked_token()?;
        assert_eq!(peeked, Peeked::EndObject, "endObject is called only where hasNext answered false in an object");
        self.pop();
        *self.path_indices.last_mut().unwrap() += 1;
        self.peeked = Peeked::None;
        Ok(())
    }

    /// `nextName()` (`JsonReader.java:925-943`). The name lands in the path before the
    /// value is read, which is why `{"a":}` reports `$.a`.
    fn next_name(&mut self) -> Result<String, Thrown> {
        let result = match self.peeked_token()? {
            Peeked::UnquotedName => self.next_unquoted_value()?,
            Peeked::SingleQuotedName => self.next_quoted_value('\'')?,
            Peeked::DoubleQuotedName => self.next_quoted_value('"')?,
            other => unreachable!("the tree adapter asks for a name only where peek said NAME, not {other:?}"),
        };
        self.peeked = Peeked::None;
        *self.path_names.last_mut().unwrap() = Some(result.clone());
        Ok(result)
    }

    /// `nextString()` (`JsonReader.java:951-977`).
    fn next_string(&mut self) -> Result<String, Thrown> {
        let result = match self.peeked_token()? {
            Peeked::Unquoted => self.next_unquoted_value()?,
            Peeked::SingleQuoted => self.next_quoted_value('\'')?,
            Peeked::DoubleQuoted => self.next_quoted_value('"')?,
            // `Long.toString(peekedLong)`, not the literal. The two agree: `peekNumber`
            // keeps a leading `0`, a `+` sign and a lone `-0` out of PEEKED_LONG.
            Peeked::Long => self.peeked_long.to_string(),
            Peeked::Number => {
                let text = java_string(&self.buffer[self.pos..self.pos + self.peeked_number_length])?;
                self.pos += self.peeked_number_length;
                text
            }
            other => unreachable!("the tree adapter asks for a string only where peek said STRING or NUMBER, not {other:?}"),
        };
        self.peeked = Peeked::None;
        *self.path_indices.last_mut().unwrap() += 1;
        Ok(result)
    }

    /// `nextBoolean()` (`JsonReader.java:984-999`).
    fn next_boolean(&mut self) -> Result<bool, Thrown> {
        let result = match self.peeked_token()? {
            Peeked::True => true,
            Peeked::False => false,
            other => unreachable!("the tree adapter asks for a boolean only where peek said BOOLEAN, not {other:?}"),
        };
        self.peeked = Peeked::None;
        *self.path_indices.last_mut().unwrap() += 1;
        Ok(result)
    }

    /// `nextNull()` (`JsonReader.java:1006-1017`).
    fn next_null(&mut self) -> Result<(), Thrown> {
        let peeked = self.peeked_token()?;
        assert_eq!(peeked, Peeked::Null, "nextNull is called only where peek answered NULL");
        self.peeked = Peeked::None;
        *self.path_indices.last_mut().unwrap() += 1;
        Ok(())
    }

    /// `doPeek()` (`JsonReader.java:582-730`).
    fn do_peek(&mut self) -> Result<Peeked, Thrown> {
        let peek_stack = *self.stack.last().unwrap();
        match peek_stack {
            Scope::EmptyArray => *self.stack.last_mut().unwrap() = Scope::NonemptyArray,
            Scope::NonemptyArray => match self.next_non_whitespace()? {
                ']' => return Ok(self.set_peeked(Peeked::EndArray)),
                ';' => self.check_lenient()?,
                ',' => {}
                _ => return Err(self.syntax_error("Unterminated array")),
            },
            Scope::EmptyObject | Scope::NonemptyObject => {
                *self.stack.last_mut().unwrap() = Scope::DanglingName;
                if peek_stack == Scope::NonemptyObject {
                    match self.next_non_whitespace()? {
                        '}' => return Ok(self.set_peeked(Peeked::EndObject)),
                        ';' => self.check_lenient()?,
                        ',' => {}
                        _ => return Err(self.syntax_error("Unterminated object")),
                    }
                }
                return match self.next_non_whitespace()? {
                    '"' => Ok(self.set_peeked(Peeked::DoubleQuotedName)),
                    '\'' => {
                        self.check_lenient()?;
                        Ok(self.set_peeked(Peeked::SingleQuotedName))
                    }
                    '}' if peek_stack != Scope::NonemptyObject => Ok(self.set_peeked(Peeked::EndObject)),
                    '}' => Err(self.syntax_error("Expected name")),
                    c => {
                        self.check_lenient()?;
                        // Don't consume the first character in an unquoted string.
                        self.pos -= 1;
                        if self.is_literal(c)? { Ok(self.set_peeked(Peeked::UnquotedName)) } else { Err(self.syntax_error("Expected name")) }
                    }
                };
            }
            Scope::DanglingName => {
                *self.stack.last_mut().unwrap() = Scope::NonemptyObject;
                match self.next_non_whitespace()? {
                    ':' => {}
                    '=' => {
                        self.check_lenient()?;
                        if self.pos < self.buffer.len() && self.char_at(self.pos) == '>' {
                            self.pos += 1;
                        }
                    }
                    _ => return Err(self.syntax_error("Expected ':'")),
                }
            }
            Scope::EmptyDocument => {
                if self.strictness == Strictness::Lenient {
                    self.consume_non_execute_prefix()?;
                }
                *self.stack.last_mut().unwrap() = Scope::NonemptyDocument;
            }
            Scope::NonemptyDocument => match self.next_non_whitespace_or_eof()? {
                None => return Ok(self.set_peeked(Peeked::Eof)),
                Some(_) => {
                    self.check_lenient()?;
                    self.pos -= 1;
                }
            },
        }

        match self.next_non_whitespace()? {
            ']' if peek_stack == Scope::EmptyArray => return Ok(self.set_peeked(Peeked::EndArray)),
            // `",]"` and the array separators fall through together: in a lenient array a
            // zero-length literal is a `null`.
            ']' | ';' | ',' => {
                if peek_stack == Scope::EmptyArray || peek_stack == Scope::NonemptyArray {
                    self.check_lenient()?;
                    self.pos -= 1;
                    return Ok(self.set_peeked(Peeked::Null));
                }
                return Err(self.syntax_error("Unexpected value"));
            }
            '\'' => {
                self.check_lenient()?;
                return Ok(self.set_peeked(Peeked::SingleQuoted));
            }
            '"' => return Ok(self.set_peeked(Peeked::DoubleQuoted)),
            '[' => return Ok(self.set_peeked(Peeked::BeginArray)),
            '{' => return Ok(self.set_peeked(Peeked::BeginObject)),
            // Don't consume the first character in a literal value.
            _ => self.pos -= 1,
        }

        if let Some(peeked) = self.peek_keyword()? {
            return Ok(peeked);
        }
        if let Some(peeked) = self.peek_number()? {
            return Ok(peeked);
        }
        if !self.is_literal(self.char_at(self.pos))? {
            return Err(self.syntax_error("Expected value"));
        }
        self.check_lenient()?;
        Ok(self.set_peeked(Peeked::Unquoted))
    }

    /// `peeked = p; return peeked;`.
    fn set_peeked(&mut self, peeked: Peeked) -> Peeked {
        self.peeked = peeked;
        peeked
    }

    /// `peekKeyword()` (`JsonReader.java:732-780`). Any capitalisation matches, because only
    /// STRICT turns that off.
    fn peek_keyword(&mut self) -> Result<Option<Peeked>, Thrown> {
        let (keyword, upper, peeking) = match self.char_at(self.pos) {
            't' | 'T' => ("true", "TRUE", Peeked::True),
            'f' | 'F' => ("false", "FALSE", Peeked::False),
            'n' | 'N' => ("null", "NULL", Peeked::Null),
            _ => return Ok(None),
        };
        let length = keyword.len();
        for (i, (expected, expected_upper)) in keyword.chars().zip(upper.chars()).enumerate() {
            if self.pos + i >= self.buffer.len() {
                return Ok(None);
            }
            let c = self.char_at(self.pos + i);
            if c != expected && c != expected_upper {
                return Ok(None);
            }
        }
        // Don't match trues, falsey or nullsoft!
        if self.pos + length < self.buffer.len() && self.is_literal(self.char_at(self.pos + length))? {
            return Ok(None);
        }
        self.pos += length;
        Ok(Some(self.set_peeked(peeking)))
    }

    /// `peekNumber()` (`JsonReader.java:782-892`). A literal of [`BUFFER_SIZE`] units or
    /// more does not fit the window gson decodes numbers out of, and becomes an unquoted
    /// string; the long accumulator wraps around as Java's does, which is what `fitsInLong`
    /// is there to notice.
    fn peek_number(&mut self) -> Result<Option<Peeked>, Thrown> {
        let mut value: i64 = 0;
        let mut negative = false;
        let mut fits_in_long = true;
        let mut last = NumberChar::None;
        let mut i = 0;

        loop {
            if i == BUFFER_SIZE {
                return Ok(None);
            }
            if self.pos + i == self.buffer.len() {
                break;
            }
            match self.char_at(self.pos + i) {
                '-' => match last {
                    NumberChar::None => {
                        negative = true;
                        last = NumberChar::Sign;
                    }
                    NumberChar::ExpE => last = NumberChar::ExpSign,
                    _ => return Ok(None),
                },
                '+' if last == NumberChar::ExpE => last = NumberChar::ExpSign,
                '+' => return Ok(None),
                'e' | 'E' if last == NumberChar::Digit || last == NumberChar::FractionDigit => last = NumberChar::ExpE,
                'e' | 'E' => return Ok(None),
                '.' if last == NumberChar::Digit => last = NumberChar::Decimal,
                '.' => return Ok(None),
                c if !c.is_ascii_digit() => {
                    if self.is_literal(c)? {
                        return Ok(None);
                    }
                    break;
                }
                c => {
                    let digit = i64::from(c as u8 - b'0');
                    match last {
                        NumberChar::None | NumberChar::Sign => {
                            value = -digit;
                            last = NumberChar::Digit;
                        }
                        NumberChar::Digit => {
                            // A leading '0' prefix is not allowed, since it could be octal.
                            if value == 0 {
                                return Ok(None);
                            }
                            let new_value = value.wrapping_mul(10).wrapping_sub(digit);
                            fits_in_long &= value > MIN_INCOMPLETE_INTEGER || (value == MIN_INCOMPLETE_INTEGER && new_value < value);
                            value = new_value;
                        }
                        NumberChar::Decimal => last = NumberChar::FractionDigit,
                        NumberChar::ExpE | NumberChar::ExpSign => last = NumberChar::ExpDigit,
                        NumberChar::FractionDigit | NumberChar::ExpDigit => {}
                    }
                }
            }
            i += 1;
        }

        // `-0` is left out of PEEKED_LONG so that it can still be read as the double -0.0,
        // and `Long.MIN_VALUE` without its sign would overflow on the way back to positive.
        if last == NumberChar::Digit && fits_in_long && (value != i64::MIN || negative) && (value != 0 || !negative) {
            self.peeked_long = if negative { value } else { value.wrapping_neg() };
            self.pos += i;
            return Ok(Some(self.set_peeked(Peeked::Long)));
        }
        if last == NumberChar::Digit || last == NumberChar::FractionDigit || last == NumberChar::ExpDigit {
            self.peeked_number_length = i;
            return Ok(Some(self.set_peeked(Peeked::Number)));
        }
        Ok(None)
    }

    /// `isLiteral(char)` (`JsonReader.java:894-918`), which is also the character set
    /// `nextUnquotedValue` and `skipUnquotedValue` stop at. The five characters that only
    /// lenient parsing allows raise the error before the answer comes back.
    fn is_literal(&self, c: char) -> Result<bool, Thrown> {
        match c {
            '/' | '\\' | ';' | '#' | '=' => {
                self.check_lenient()?;
                Ok(false)
            }
            '{' | '}' | '[' | ']' | ':' | ',' | ' ' | '\t' | '\u{c}' | '\r' | '\n' => Ok(false),
            _ => Ok(true),
        }
    }

    /// `nextQuotedValue(char)` (`JsonReader.java:1125-1179`). A raw line feed inside the
    /// string is kept and counts a line; only STRICT refuses the control characters.
    fn next_quoted_value(&mut self, quote: char) -> Result<String, Thrown> {
        let mut builder: Vec<u16> = Vec::new();
        let mut start = self.pos;
        while self.pos < self.buffer.len() {
            let c = self.char_at(self.pos);
            self.pos += 1;
            if c == quote {
                builder.extend_from_slice(&self.buffer[start..self.pos - 1]);
                return java_string(&builder);
            } else if c == '\\' {
                builder.extend_from_slice(&self.buffer[start..self.pos - 1]);
                let escaped = self.read_escape_character()?;
                builder.push(escaped);
                start = self.pos;
            } else if c == '\n' {
                self.line_number += 1;
                self.line_start = self.pos;
            }
        }
        Err(self.syntax_error("Unterminated string"))
    }

    /// `nextUnquotedValue()` (`JsonReader.java:1183-1239`).
    fn next_unquoted_value(&mut self) -> Result<String, Thrown> {
        let mut i = 0;
        while self.pos + i < self.buffer.len() && self.is_literal(self.char_at(self.pos + i))? {
            i += 1;
        }
        let result = java_string(&self.buffer[self.pos..self.pos + i])?;
        self.pos += i;
        Ok(result)
    }

    /// `readEscapeCharacter()` (`JsonReader.java:1745-1809`). It answers one UTF-16 unit, so
    /// an astral character takes a `\u` pair and a lone `\ud800` is kept as it is.
    fn read_escape_character(&mut self) -> Result<u16, Thrown> {
        if self.pos == self.buffer.len() {
            return Err(self.syntax_error("Unterminated escape sequence"));
        }
        let escaped = self.char_at(self.pos);
        self.pos += 1;
        match escaped {
            'u' => {
                if self.pos + 4 > self.buffer.len() {
                    return Err(self.syntax_error("Unterminated escape sequence"));
                }
                let mut result: u16 = 0;
                for i in self.pos..self.pos + 4 {
                    let c = self.char_at(i);
                    result <<= 4;
                    match c.to_digit(16) {
                        Some(digit) => result += digit as u16,
                        None => {
                            let shown = java_string(&self.buffer[self.pos..self.pos + 4])?;
                            return Err(self.syntax_error(&format!("Malformed Unicode escape \\u{shown}")));
                        }
                    }
                }
                self.pos += 4;
                Ok(result)
            }
            't' => Ok(u16::from(b'\t')),
            'b' => Ok(0x08),
            'n' => Ok(u16::from(b'\n')),
            'r' => Ok(u16::from(b'\r')),
            'f' => Ok(0x0c),
            // An escaped line feed stands for a line feed, and counts a line.
            '\n' => {
                self.line_number += 1;
                self.line_start = self.pos;
                Ok(u16::from(b'\n'))
            }
            '\'' | '"' | '\\' | '/' => Ok(escaped as u16),
            _ => Err(self.syntax_error("Invalid escape sequence")),
        }
    }

    /// `nextNonWhitespace(true)`: the character is left at `buffer[pos - 1]`, so the caller
    /// can push it back by decrementing `pos`.
    fn next_non_whitespace(&mut self) -> Result<char, Thrown> {
        match self.next_non_whitespace_or_eof()? {
            Some(c) => Ok(c),
            None => Err(self.eof_error()),
        }
    }

    /// `nextNonWhitespace(false)` (`JsonReader.java:1522-1609`). A form feed is not
    /// whitespace here, and a carriage return is whitespace but starts no line.
    fn next_non_whitespace_or_eof(&mut self) -> Result<Option<char>, Thrown> {
        while self.pos < self.buffer.len() {
            let c = self.char_at(self.pos);
            self.pos += 1;
            match c {
                '\n' => {
                    self.line_number += 1;
                    self.line_start = self.pos;
                }
                ' ' | '\r' | '\t' => {}
                '/' => {
                    // A '/' at the very end of the document is a character of its own, not
                    // the start of a comment, and it is handed back without a strictness
                    // check.
                    if self.pos == self.buffer.len() {
                        return Ok(Some(c));
                    }
                    self.check_lenient()?;
                    match self.char_at(self.pos) {
                        '*' => {
                            self.pos += 1;
                            if !self.skip_to_comment_end() {
                                return Err(self.syntax_error("Unterminated comment"));
                            }
                            self.pos += 2;
                        }
                        '/' => {
                            self.pos += 1;
                            self.skip_to_end_of_line();
                        }
                        _ => return Ok(Some(c)),
                    }
                }
                '#' => {
                    self.check_lenient()?;
                    self.skip_to_end_of_line();
                }
                _ => return Ok(Some(c)),
            }
        }
        Ok(None)
    }

    /// `checkLenient()` (`JsonReader.java:1611-1616`).
    fn check_lenient(&self) -> Result<(), Thrown> {
        if self.strictness != Strictness::Lenient {
            return Err(self.syntax_error("Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON"));
        }
        Ok(())
    }

    /// `skipToEndOfLine()` (`JsonReader.java:1622-1633`): a `\r` ends the comment but is
    /// left to count no line, so the `\n` of a `\r\n` is counted later as whitespace.
    fn skip_to_end_of_line(&mut self) {
        while self.pos < self.buffer.len() {
            let c = self.char_at(self.pos);
            self.pos += 1;
            if c == '\n' {
                self.line_number += 1;
                self.line_start = self.pos;
                break;
            }
            if c == '\r' {
                break;
            }
        }
    }

    /// `skipTo("*/")` (`JsonReader.java:1638-1655`). It stops at the last position where two
    /// units are still available, which is the column an unterminated comment reports.
    fn skip_to_comment_end(&mut self) -> bool {
        while self.pos + 2 <= self.buffer.len() {
            if self.char_at(self.pos) == '\n' {
                self.line_number += 1;
                self.line_start = self.pos + 1;
                self.pos += 1;
                continue;
            }
            if self.char_at(self.pos) == '*' && self.char_at(self.pos + 1) == '/' {
                return true;
            }
            self.pos += 1;
        }
        false
    }

    /// `consumeNonExecutePrefix()` (`JsonReader.java:1835-1856`), which runs on a lenient
    /// document before its first token and throws on an empty one.
    fn consume_non_execute_prefix(&mut self) -> Result<(), Thrown> {
        self.next_non_whitespace()?;
        self.pos -= 1;
        if self.pos + 5 > self.buffer.len() {
            return Ok(());
        }
        if self.buffer[self.pos..self.pos + 5] == NON_EXECUTE_PREFIX {
            self.pos += 5;
        }
        Ok(())
    }

    /// `push(int)` (`JsonReader.java:1468-1482`). The nesting limit's message carries no
    /// troubleshooting URL, because it is built without `syntaxError`. gson counts
    /// `stackSize - 1`, leaving out the document the stack always starts with.
    fn push(&mut self, new_top: Scope) -> Result<(), Thrown> {
        if self.stack.len() > NESTING_LIMIT {
            return Err(Thrown::Malformed(format!("Nesting limit {NESTING_LIMIT} reached{}", self.location_string())));
        }
        self.stack.push(new_top);
        self.path_names.push(None);
        self.path_indices.push(0);
        Ok(())
    }

    /// `stackSize--` with the name at the old top freed, which is why an object that has
    /// ended leaves no name behind in the path.
    fn pop(&mut self) {
        self.stack.pop();
        self.path_names.pop();
        self.path_indices.pop();
    }

    /// `new EOFException("End of input" + locationString())`.
    fn eof_error(&self) -> Thrown {
        Thrown::Eof(format!("End of input{}", self.location_string()))
    }

    /// `syntaxError(String)` (`JsonReader.java:1815-1818`).
    fn syntax_error(&self, message: &str) -> Thrown {
        Thrown::Malformed(format!(
            "{message}{}\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json",
            self.location_string()
        ))
    }

    /// `locationString()` (`JsonReader.java:1662-1666`). The column is the one *after* the
    /// token the error is about, because `pos` has already consumed it.
    fn location_string(&self) -> String {
        format!(" at line {} column {} path {}", self.line_number + 1, self.pos - self.line_start + 1, self.path())
    }

    /// `getPath(false)` (`JsonReader.java:1668-1699`). An array scope shows the index of the
    /// element to come, an object scope its last name, and a scope whose name has not been
    /// read yet shows nothing after the dot.
    fn path(&self) -> String {
        let mut result = String::from("$");
        for (i, scope) in self.stack.iter().enumerate() {
            match scope {
                Scope::EmptyArray | Scope::NonemptyArray => {
                    result.push('[');
                    result.push_str(&self.path_indices[i].to_string());
                    result.push(']');
                }
                Scope::EmptyObject | Scope::DanglingName | Scope::NonemptyObject => {
                    result.push('.');
                    if let Some(name) = &self.path_names[i] {
                        result.push_str(name);
                    }
                }
                Scope::EmptyDocument | Scope::NonemptyDocument => {}
            }
        }
        result
    }
}

/// A Java `String` made of the UTF-16 units gson collected. Only a `\u` escape can leave an
/// unpaired surrogate among them, because a document's own text was decoded from a
/// well-formed `String`; two escapes that spell an astral character are joined here as Java
/// joins them.
fn java_string(units: &[u16]) -> Result<String, Thrown> {
    String::from_utf16(units).map_err(|_| Thrown::Unpaired)
}

#[cfg(test)]
mod tests {
    use super::{Failure, parse_reader};

    /// What `JsonParser.parseString` answers on the pinned jar, as
    /// `JsonElement.toString()`: the quotes tell a `JsonPrimitive` string from a number.
    /// A failure becomes the message the module would report, so the expectations below are
    /// the text TLC prints.
    fn parsed(text: &str) -> Result<String, String> {
        parse_reader(text).map(|node| node.to_json()).map_err(|failure| match failure {
            Failure::Thrown(message) => message,
            Failure::UnpairedSurrogate => "<a document this port cannot represent>".to_string(),
        })
    }

    /// The message of the `JsonSyntaxException` built from a `MalformedJsonException`.
    fn malformed(message: &str) -> Result<String, String> {
        Err(format!(
            "com.google.gson.stream.MalformedJsonException: {message}\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json"
        ))
    }

    /// `checkLenient`'s message, which every piece of trailing data gets, because the
    /// document-level peek that finds it runs under LEGACY_STRICT again.
    fn not_lenient(location: &str) -> Result<String, String> {
        malformed(&format!("Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON{location}"))
    }

    /// The message of the `JsonSyntaxException` built from an `EOFException`.
    fn end_of_input(location: &str) -> Result<String, String> {
        Err(format!("java.io.EOFException: End of input{location}"))
    }

    /// The whole text of two errors, spelled out once, so the helpers above cannot agree
    /// with the port on a wrong URL or class name.
    #[test]
    fn error_messages_are_the_wrapped_causes_tostring() {
        assert_eq!(
            parsed("[1] [2]"),
            Err("com.google.gson.stream.MalformedJsonException: Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON at line 1 column 6 path $\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json".to_string())
        );
        assert_eq!(parsed("[1,"), Err("java.io.EOFException: End of input at line 1 column 4 path $[1]".to_string()));
    }

    /// gson answers a Java `String` holding the lone code unit; this port has no such value
    /// and says so, so a spec reading such a document stops instead of being handed a
    /// different string. A pair that spells an astral character is not affected.
    #[test]
    fn an_unpaired_surrogate_escape_has_no_value() {
        for text in [r#"["\ud800"]"#, r#"["\ud83dx"]"#, r#"{"\ud800":1}"#, r#"["\ude00\ud83d"]"#] {
            assert!(matches!(parse_reader(text), Err(Failure::UnpairedSurrogate)), "{text}");
        }
        assert_eq!(parsed(r#"["\ud83d\ude00"]"#), Ok("[\"\u{1f600}\"]".to_string()));
    }

    /// The documents gson's lenient reader accepts, and the trees it builds.
    #[test]
    fn accepts_what_gson_accepts() {
        let cases: &[(&str, &str)] = &[
            ("[1,2]", "[1,2]"),
            ("[]", "[]"),
            ("{}", "{}"),
            ("[{}]", "[{}]"),
            ("null", "null"),
            ("[null]", "[null]"),
            ("[true,false]", "[true,false]"),
            ("", "null"),
            ("   \n ", "null"),
            ("{'a':1}", r#"{"a":1}"#),
            ("{a:1}", r#"{"a":1}"#),
            ("// hi\n[1]", "[1]"),
            ("/* hi */ [1]", "[1]"),
            ("# hi\n[1]", "[1]"),
            ("// hi", "null"),
            ("/* hi */", "null"),
            ("# hi", "null"),
            (r#"{"a":1,"a":2}"#, r#"{"a":2}"#),
            (r#"{"a":1,'a':2,a:3}"#, r#"{"a":3}"#),
            (r#"{"":1}"#, r#"{"":1}"#),
            (r#"{"a":{"b":[1,2]}}"#, r#"{"a":{"b":[1,2]}}"#),
            (r#"{"a":[1,{"b":[2]}]}"#, r#"{"a":[1,{"b":[2]}]}"#),
            (r#"{"a":[{"b":null}],"c":true}"#, r#"{"a":[{"b":null}],"c":true}"#),
            ("[1;2]", "[1,2]"),
            ("[1,]", "[1,null]"),
            ("[1;]", "[1,null]"),
            ("[,1]", "[null,1]"),
            ("[1,,2]", "[1,null,2]"),
            ("[,]", "[null,null]"),
            ("{1:2}", r#"{"1":2}"#),
            ("{-1:2}", r#"{"-1":2}"#),
            ("{true:1}", r#"{"true":1}"#),
            ("{null:1}", r#"{"null":1}"#),
            (r#"{"a"=1}"#, r#"{"a":1}"#),
            (r#"{"a"=>1}"#, r#"{"a":1}"#),
            (r#"{"a":1;"b":2}"#, r#"{"a":1,"b":2}"#),
            (r#"{ "a" : 1 }"#, r#"{"a":1}"#),
            (" \t\r\n[ 1 , 2 ] ", "[1,2]"),
            ("[1,2]\n", "[1,2]"),
            ("[1,2]\n\n", "[1,2]"),
            ("[1,2] ", "[1,2]"),
            ("abc", r#""abc""#),
            ("[abc,def]", r#"["abc","def"]"#),
            ("TRUE", "true"),
            ("False", "false"),
            ("NULL", "null"),
            ("nULl", "null"),
            ("trues", r#""trues""#),
            ("nullsoft", r#""nullsoft""#),
            (r#"["a'b"]"#, r#"["a'b"]"#),
            (r#"['a"b']"#, r#"["a\"b"]"#),
            ("['']", r#"[""]"#),
            // The non-execute prefix, which only lenient parsing drops.
            (")]}'\n[1]", "[1]"),
            ("  )]}'\n[1]", "[1]"),
            (")]}'\n", "null"),
            ("\u{feff}[1]", "[1]"),
            ("\u{feff}", "null"),
            // The trailing-data check is skipped for a JsonNull.
            ("null 1", "null"),
            ("null null", "null"),
            ("NULL x", "null"),
        ];
        for (text, want) in cases {
            assert_eq!(parsed(text), Ok((*want).to_string()), "{text:?}");
        }
    }

    /// Numbers: what stays a number, what becomes a string, and the literal that is kept.
    #[test]
    fn numbers_keep_their_literal() {
        let cases: &[(&str, &str)] = &[
            ("1.5", "1.5"),
            ("1e2", "1e2"),
            ("2.0", "2.0"),
            ("9999999999", "9999999999"),
            ("2147483647", "2147483647"),
            ("-2147483648", "-2147483648"),
            ("9223372036854775807", "9223372036854775807"),
            ("-9223372036854775808", "-9223372036854775808"),
            ("9223372036854775808", "9223372036854775808"),
            ("-0", "-0"),
            ("-0.0", "-0.0"),
            ("0.5", "0.5"),
            ("1E+2", "1E+2"),
            ("1e-2", "1e-2"),
            // Not numbers, so unquoted strings.
            ("01", r#""01""#),
            ("00", r#""00""#),
            ("+1", r#""+1""#),
            ("0x10", r#""0x10""#),
            (".5", r#"".5""#),
            ("5.", r#""5.""#),
            ("1.2.3", r#""1.2.3""#),
            ("1e", r#""1e""#),
            ("1e+", r#""1e+""#),
            ("-", r#""-""#),
            ("--1", r#""--1""#),
            ("NaN", r#""NaN""#),
            ("Infinity", r#""Infinity""#),
            ("-Infinity", r#""-Infinity""#),
            ("[NaN,Infinity]", r#"["NaN","Infinity"]"#),
        ];
        for (text, want) in cases {
            assert_eq!(parsed(text), Ok((*want).to_string()), "{text:?}");
        }
    }

    /// A numeric literal of `BUFFER_SIZE` units or more does not fit the window `peekNumber`
    /// decodes out of, so it comes back as a string however it is placed in the document.
    #[test]
    fn a_number_longer_than_the_buffer_becomes_a_string() {
        for (len, number) in [(1022, true), (1023, true), (1024, false), (1025, false)] {
            let digits = "9".repeat(len);
            let quoted = format!("\"{digits}\"");
            let want = if number { &digits } else { &quoted };
            assert_eq!(parsed(&digits), Ok(want.clone()), "{len} digits bare");
            assert_eq!(parsed(&format!("[{digits}]")), Ok(format!("[{want}]")), "{len} digits in an array");
            assert_eq!(parsed(&format!("[   {digits}]")), Ok(format!("[{want}]")), "{len} digits behind padding");
            let fraction = format!("0.{}", "9".repeat(len - 2));
            let want = if number { fraction.clone() } else { format!("\"{fraction}\"") };
            assert_eq!(parsed(&fraction), Ok(want), "{len} units as a fraction");
        }
        // Strings and the location arithmetic are not measured against the window.
        let padded = format!("{}[1] x", " ".repeat(3000));
        assert_eq!(parsed(&padded), not_lenient(" at line 1 column 3006 path $"));
        let lines = format!("{}[1] x", "\n".repeat(3000));
        assert_eq!(parsed(&lines), not_lenient(" at line 3001 column 6 path $"));
    }

    /// Strings: the escapes gson reads and the characters it lets through unescaped.
    #[test]
    fn strings_unescape_as_gson_does() {
        let cases: &[(&str, &str)] = &[
            (r#"["\u3042"]"#, "[\"\u{3042}\"]"),
            (r#"["\u00E9"]"#, "[\"\u{e9}\"]"),
            (r#"["\ud83d\ude00"]"#, "[\"\u{1f600}\"]"),
            (r#"["a\'b"]"#, r#"["a'b"]"#),
            (r#"["a\/b"]"#, r#"["a/b"]"#),
            (r#"["a\\b"]"#, r#"["a\\b"]"#),
            (r#"["\b\f\n\r\t"]"#, r#"["\b\f\n\r\t"]"#),
            ("[\"a\u{1}b\"]", r#"["a\u0001b"]"#),
            ("[\"a\u{0}b\"]", r#"["a\u0000b"]"#),
            ("[\"a\nb\"]", r#"["a\nb"]"#),
            ("[\"a\\\nb\"]", r#"["a\nb"]"#),
            // A vertical tab is a literal character here, and the writer escapes it again.
            ("[\u{b}]", r#"["\u000b"]"#),
            ("[\u{feff}1]", "[\"\u{feff}1\"]"),
            (r#"{"a\"b":1}"#, r#"{"a\"b":1}"#),
            (r#"{"\u0041":1}"#, r#"{"A":1}"#),
            (r#"{"a.b":1}"#, r#"{"a.b":1}"#),
        ];
        for (text, want) in cases {
            assert_eq!(parsed(text), Ok((*want).to_string()), "{text:?}");
        }
    }

    /// Trailing data, which the document-level peek reports under LEGACY_STRICT.
    #[test]
    fn trailing_data_is_not_lenient() {
        let cases: &[(&str, &str)] = &[
            ("[1] [2]", " at line 1 column 6 path $"),
            ("[1] x", " at line 1 column 6 path $"),
            ("1 2", " at line 1 column 4 path $"),
            ("[1] // hi", " at line 1 column 6 path $"),
            ("[1] /* hi */", " at line 1 column 6 path $"),
            ("[1] /*", " at line 1 column 6 path $"),
            ("[1]]", " at line 1 column 5 path $"),
            (r#"{"a":1}}"#, " at line 1 column 9 path $"),
            ("[null] x", " at line 1 column 9 path $"),
            ("[1,2]\tx", " at line 1 column 8 path $"),
            ("[1,2]\u{0}", " at line 1 column 7 path $"),
            ("[1]\nx", " at line 2 column 2 path $"),
            ("[1]\r\nx", " at line 2 column 2 path $"),
            ("[1]\rx", " at line 1 column 6 path $"),
            // A '\r' ends a line comment without counting a line; the '\n' of a "\r\n" is
            // counted afterwards, as whitespace.
            ("// hi\r[1] x", " at line 1 column 12 path $"),
            ("// hi\r\n[1] x", " at line 2 column 6 path $"),
            ("// hi\n[1] x", " at line 2 column 6 path $"),
            ("/* a\nb */ [1] x", " at line 2 column 11 path $"),
            ("[\"a\nb\"] x", " at line 2 column 6 path $"),
            ("\u{feff}[1] x", " at line 1 column 6 path $"),
            // Only the first mark is dropped, so a second one is trailing data.
            ("\u{feff}\u{feff}[1]", " at line 1 column 3 path $"),
            // The non-execute prefix without its line feed is not a prefix.
            (")]}'[1]", " at line 1 column 3 path $"),
        ];
        for (text, location) in cases {
            assert_eq!(parsed(text), not_lenient(location), "{text:?}");
        }
    }

    /// The reader's own syntax errors, with the line, column and path each reports.
    #[test]
    fn syntax_errors_match_gson() {
        let cases: &[(&str, &str)] = &[
            (r#"{"a" 1}"#, "Expected ':' at line 1 column 7 path $.a"),
            ("{a b:1}", "Expected ':' at line 1 column 5 path $.a"),
            (r#"["\q"]"#, "Invalid escape sequence at line 1 column 5 path $[0]"),
            (r#"{"a":1,}"#, "Expected name at line 1 column 9 path $.a"),
            ("{,}", "Expected name at line 1 column 2 path $."),
            ("{\u{c}\"a\":1}", "Expected name at line 1 column 2 path $."),
            (r#"{"a":}"#, "Expected value at line 1 column 6 path $.a"),
            (r#"{"a.b":}"#, "Expected value at line 1 column 8 path $.a.b"),
            (r#"{"a[0]":}"#, "Expected value at line 1 column 9 path $.a[0]"),
            ("{\"a\nb\":}", "Expected value at line 2 column 4 path $.a\nb"),
            (r#"{"a":{"b":[1,}}"#, "Expected value at line 1 column 14 path $.a.b[1]"),
            (r#"{"a":{"b":[1,2}}"#, "Unterminated array at line 1 column 16 path $.a.b[2]"),
            ("[[1,2],}", "Expected value at line 1 column 8 path $[1]"),
            ("[[1,2],3,}", "Expected value at line 1 column 10 path $[2]"),
            (r#"[{"a":1},}"#, "Expected value at line 1 column 10 path $[1]"),
            ("[1}", "Unterminated array at line 1 column 4 path $[1]"),
            ("[1 2]", "Unterminated array at line 1 column 5 path $[1]"),
            ("[a:b]", "Unterminated array at line 1 column 4 path $[1]"),
            ("[a=b]", "Unterminated array at line 1 column 4 path $[1]"),
            (r"[a\b]", "Unterminated array at line 1 column 4 path $[1]"),
            ("[\"\u{1f600}\" x]", "Unterminated array at line 1 column 8 path $[1]"),
            ("[1,\r\n2 3]", "Unterminated array at line 2 column 4 path $[2]"),
            ("[1,\r2 3]", "Unterminated array at line 1 column 8 path $[2]"),
            ("[1,\n2 3]", "Unterminated array at line 2 column 4 path $[2]"),
            (r#"{"a":1 2}"#, "Unterminated object at line 1 column 9 path $.a"),
            (r#"{"a":1 "b":2}"#, "Unterminated object at line 1 column 9 path $.a"),
            ("[:]", "Expected value at line 1 column 2 path $[0]"),
            ("[/]", "Expected value at line 1 column 2 path $[0]"),
            ("/", "Expected value at line 1 column 1 path $"),
            (":", "Expected value at line 1 column 1 path $"),
            ("}", "Expected value at line 1 column 1 path $"),
            ("\u{c}[1]", "Expected value at line 1 column 1 path $"),
            ("[1,\u{c}2]", "Expected value at line 1 column 4 path $[1]"),
            (",", "Unexpected value at line 1 column 2 path $"),
            ("]", "Unexpected value at line 1 column 2 path $"),
            (r#""abc"#, "Unterminated string at line 1 column 5 path $"),
            (r#"["ab]"#, "Unterminated string at line 1 column 6 path $[0]"),
            ("[\"ab\n", "Unterminated string at line 2 column 1 path $[0]"),
            ("['ab", "Unterminated string at line 1 column 5 path $[0]"),
            (r#"["a\u1234"#, "Unterminated string at line 1 column 10 path $[0]"),
            ("/* hi", "Unterminated comment at line 1 column 5 path $"),
            ("/*", "Unterminated comment at line 1 column 3 path $"),
            ("[1 /* x", "Unterminated comment at line 1 column 7 path $[1]"),
            ("[\"a\\", "Unterminated escape sequence at line 1 column 5 path $[0]"),
            (r#"["a\u00"#, "Unterminated escape sequence at line 1 column 6 path $[0]"),
            (r#"["a\u123"#, "Unterminated escape sequence at line 1 column 6 path $[0]"),
            (r#"["\u00"]"#, "Malformed Unicode escape \\u00\"] at line 1 column 5 path $[0]"),
            (r#"["\u0g12"]"#, "Malformed Unicode escape \\u0g12 at line 1 column 5 path $[0]"),
        ];
        for (text, message) in cases {
            assert_eq!(parsed(text), malformed(message), "{text:?}");
        }
    }

    /// A document that ends too early: an EOF before any token is a `JsonNull`, an EOF after
    /// one is an `EOFException` whose path says where the reader stood.
    #[test]
    fn truncated_documents_report_end_of_input() {
        let cases: &[(&str, &str)] = &[
            ("{", " at line 1 column 2 path $."),
            ("[1,", " at line 1 column 4 path $[1]"),
            ("[[1]", " at line 1 column 5 path $[1]"),
            (r#"{"a":{"b":[1,2]"#, " at line 1 column 16 path $.a.b"),
            (r#"{"a":{"b":"#, " at line 1 column 11 path $.a.b"),
            (r#"{"a":{"#, " at line 1 column 7 path $.a."),
            (r#"{"a":1,"#, " at line 1 column 8 path $.a"),
            (r#"{"a":1"#, " at line 1 column 7 path $.a"),
            (r#"{"a""#, " at line 1 column 5 path $.a"),
            (r#"{"a":"#, " at line 1 column 6 path $.a"),
            (r#"{"a"="#, " at line 1 column 6 path $.a"),
            (r#"{"a"=>"#, " at line 1 column 7 path $.a"),
            (r#"{"a":1,"b""#, " at line 1 column 11 path $.b"),
            // '#' starts a comment inside an array, so the rest of the line is skipped.
            ("[a#b]", " at line 1 column 6 path $[1]"),
            ("\n\n[1,\n", " at line 4 column 1 path $[1]"),
            ("\t\t[1,", " at line 1 column 6 path $[1]"),
        ];
        for (text, location) in cases {
            assert_eq!(parsed(text), end_of_input(location), "{text:?}");
        }
    }

    /// `push`'s nesting limit, which is what bounds the depth of the tree the adapter
    /// builds. Its message carries the location but no troubleshooting URL.
    #[test]
    fn nesting_limit_is_255_containers() {
        for depth in [254, 255] {
            let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
            let want = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(parsed(&text), Ok(want), "{depth} arrays");
        }
        for depth in [256, 300] {
            let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
            let want = format!("Nesting limit 255 reached at line 1 column 257 path ${}", "[0]".repeat(255));
            assert_eq!(parsed(&text), Err(format!("com.google.gson.stream.MalformedJsonException: {want}")), "{depth} arrays");
        }
        let text = format!("{}1{}", r#"{"a":"#.repeat(256), "}".repeat(256));
        let want = format!("Nesting limit 255 reached at line 1 column 1277 path ${}", ".a".repeat(255));
        assert_eq!(parsed(&text), Err(format!("com.google.gson.stream.MalformedJsonException: {want}")), "256 objects");
    }
}

