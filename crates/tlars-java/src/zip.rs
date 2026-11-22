//! `java.util.zip`: `CRC32`, and the gzip container `GZIPOutputStream` writes and
//! `GZIPInputStream` reads.
//!
//! TLC wraps every value stream in gzip (`ValueOutputStream(File, true)`), so a trace file
//! tlars writes has to be one `GZIPInputStream` accepts and the other way round.
//!
//! What is byte for byte Java's: the ten header bytes (OS is 255, as the JDK writes it since
//! JDK 16, not the 0 the class' javadoc shows) and the CRC-32 / ISIZE trailer. What is not:
//! the DEFLATE payload. [`gzip`] emits stored blocks, because which Huffman codes zlib's
//! deflate picks for a given input is not part of the format TLC defines, and porting zlib's
//! match finder would buy nothing a reader can observe. [`gunzip`] is exact the other way: it
//! reads every stream RFC 1951 allows, with the messages the pinned JDK raises
//! (`tests/golden/gzip.txt`).

/// `java.util.zip.CRC32`: the reflected CRC-32 of RFC 1952, bit by bit.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A stream `GZIPInputStream` refuses, as the exception it raises: the Java class name and
/// its message, which is what reaches a spec through TLC's `IOException` wrapping.
#[derive(Debug, PartialEq, Eq)]
pub struct GzipError {
    pub class: &'static str,
    /// `Throwable.getMessage()`, which is null for the bare `EOFException` the header read
    /// raises; `java.io.EOFException: null` is how TLC's message formatter prints that.
    pub message: Option<&'static str>,
}

impl GzipError {
    fn eof() -> GzipError {
        GzipError { class: "java.io.EOFException", message: None }
    }

    /// `GZIPInputStream.readUByte`'s "Unexpected end of ZLIB input stream", which stands for
    /// every read that runs off the end after the header.
    fn truncated() -> GzipError {
        GzipError { class: "java.io.EOFException", message: Some("Unexpected end of ZLIB input stream") }
    }

    fn zip(message: &'static str) -> GzipError {
        GzipError { class: "java.util.zip.ZipException", message: Some(message) }
    }
}

impl std::fmt::Display for GzipError {
    /// `e.getClass().getName() + ": " + e.getMessage()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.class, self.message.unwrap_or("null"))
    }
}

/// `new GZIPOutputStream(out)` fed `data` and closed: Java's header, the payload as DEFLATE
/// stored blocks, and Java's trailer.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 32);
    out.extend_from_slice(&GZIP_HEADER);
    deflate_stored(data, &mut out);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    // ISIZE: the length modulo 2^32, as `GZIPOutputStream.writeTrailer` writes it.
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// `GZIPOutputStream.writeHeader`: magic, CM = 8 (deflate), no flags, MTIME = 0, XFL = 0,
/// OS = 255 (unknown).
const GZIP_HEADER: [u8; 10] = [0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 0xff];

/// Stored (uncompressed) DEFLATE blocks of at most 65535 bytes, the last one marked final.
fn deflate_stored(data: &[u8], out: &mut Vec<u8>) {
    let mut chunks = data.chunks(0xffff);
    let count = chunks.len();
    for (i, chunk) in chunks.by_ref().enumerate() {
        out.push(u8::from(i + 1 == count));
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    if count == 0 {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
}

/// `new GZIPInputStream(in).readAllBytes()`: one gzip member, its trailer checked. Trailing
/// bytes after the member are ignored, as the stream's `readTrailer` does when what follows
/// is not another member's header.
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, GzipError> {
    // `readHeader` reads through a `CheckedInputStream` whose EOF is the bare EOFException.
    let header = bytes.get(..10).ok_or_else(GzipError::eof)?;
    if header[0] != 0x1f || header[1] != 0x8b {
        return Err(GzipError::zip("Not in GZIP format"));
    }
    if header[2] != 8 {
        return Err(GzipError::zip("Unsupported compression method"));
    }
    let flags = header[3];
    let mut pos = 10;
    if flags & FEXTRA != 0 {
        let len = u16::from(*bytes.get(pos + 1).ok_or_else(GzipError::eof)?) << 8
            | u16::from(*bytes.get(pos).ok_or_else(GzipError::eof)?);
        pos += 2 + usize::from(len);
    }
    if flags & FNAME != 0 {
        pos = skip_zero_terminated(bytes, pos)?;
    }
    if flags & FCOMMENT != 0 {
        pos = skip_zero_terminated(bytes, pos)?;
    }
    if flags & FHCRC != 0 {
        let want = u16::from(*bytes.get(pos + 1).ok_or_else(GzipError::eof)?) << 8
            | u16::from(*bytes.get(pos).ok_or_else(GzipError::eof)?);
        if want != (crc32(&bytes[..pos]) & 0xffff) as u16 {
            return Err(GzipError::zip("Corrupt GZIP header"));
        }
        pos += 2;
    }
    let (data, end) = inflate(bytes, pos)?;
    let trailer = bytes.get(end..end + 8).ok_or_else(GzipError::truncated)?;
    let crc = u32::from_le_bytes(trailer[..4].try_into().expect("four bytes"));
    let size = u32::from_le_bytes(trailer[4..].try_into().expect("four bytes"));
    if crc != crc32(&data) || size != data.len() as u32 {
        return Err(GzipError::zip("Corrupt GZIP trailer"));
    }
    Ok(data)
}

const FHCRC: u8 = 2;
const FEXTRA: u8 = 4;
const FNAME: u8 = 8;
const FCOMMENT: u8 = 16;

fn skip_zero_terminated(bytes: &[u8], from: usize) -> Result<usize, GzipError> {
    match bytes[from..].iter().position(|b| *b == 0) {
        Some(i) => Ok(from + i + 1),
        None => Err(GzipError::eof()),
    }
}

/// A DEFLATE stream from `bytes[from..]`, and the offset of the first byte after it.
/// `Inflater` stops at the final block and reports the bytes it did not consume, which is how
/// `GZIPInputStream` finds the trailer.
fn inflate(bytes: &[u8], from: usize) -> Result<(Vec<u8>, usize), GzipError> {
    let mut bits = Bits { bytes, pos: from, bit: 0 };
    let mut out = Vec::new();
    loop {
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => stored(&mut bits, &mut out)?,
            1 => block(&mut bits, &mut out, &fixed_lengths(), &fixed_distances())?,
            2 => {
                let (lengths, distances) = dynamic_tables(&mut bits)?;
                block(&mut bits, &mut out, &lengths, &distances)?;
            }
            _ => return Err(GzipError::zip("invalid block type")),
        }
        if last {
            return Ok((out, bits.consumed()));
        }
    }
}

/// The bit reader of RFC 1951: bytes in order, bits from the least significant up.
struct Bits<'a> {
    bytes: &'a [u8],
    pos: usize,
    bit: u32,
}

impl Bits<'_> {
    fn bits(&mut self, n: u32) -> Result<u32, GzipError> {
        let mut value = 0;
        for i in 0..n {
            let byte = *self.bytes.get(self.pos).ok_or_else(GzipError::truncated)?;
            value |= u32::from(byte >> self.bit & 1) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Ok(value)
    }

    /// Drops the rest of the current byte, as a stored block's header does.
    fn align(&mut self) {
        if self.bit > 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }

    /// How many bytes the stream has taken, counting a partly read one.
    fn consumed(&self) -> usize {
        self.pos + usize::from(self.bit > 0)
    }

    fn take(&mut self, n: usize) -> Result<&[u8], GzipError> {
        let bytes = self.bytes.get(self.pos..self.pos + n).ok_or_else(GzipError::truncated)?;
        self.pos += n;
        Ok(bytes)
    }
}

fn stored(bits: &mut Bits<'_>, out: &mut Vec<u8>) -> Result<(), GzipError> {
    bits.align();
    let header = bits.take(4)?;
    let len = u16::from_le_bytes([header[0], header[1]]);
    let nlen = u16::from_le_bytes([header[2], header[3]]);
    if len != !nlen {
        return Err(GzipError::zip("invalid stored block lengths"));
    }
    out.extend_from_slice(bits.take(usize::from(len))?);
    Ok(())
}

/// A canonical Huffman decoding table: how many codes have each length, and the symbols in
/// code order (`puff.c`'s representation, which decodes bit by bit without a lookup table).
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    /// Builds the table from code lengths, as zlib's `inflate_table` does: an
    /// over-subscribed set is always an error, an incomplete one only unless it is a single
    /// code in a literal/length or distance table (`left > 0 && (type == CODES || max != 1)`).
    fn new(lengths: &[u8], what: &'static str, single_may_be_incomplete: bool) -> Result<Huffman, GzipError> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[usize::from(l)] += 1;
        }
        if usize::from(counts[0]) == lengths.len() {
            return Ok(Huffman { counts, symbols: Vec::new() });
        }
        let mut left = 1i32;
        for count in counts.iter().skip(1) {
            left = (left << 1) - i32::from(*count);
            if left < 0 {
                return Err(GzipError::zip(what));
            }
        }
        let used = lengths.len() - usize::from(counts[0]);
        if left > 0 && !(single_may_be_incomplete && used == 1) {
            return Err(GzipError::zip(what));
        }
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; used];
        for (symbol, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[usize::from(offsets[usize::from(l)])] = symbol as u16;
                offsets[usize::from(l)] += 1;
            }
        }
        Ok(Huffman { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits<'_>, what: &'static str) -> Result<u16, GzipError> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.bits(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - first < count {
                return Ok(self.symbols[usize::try_from(index + code - first).expect("in range")]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(GzipError::zip(what))
    }
}

/// The fixed literal/length code of RFC 1951 section 3.2.6.
fn fixed_lengths() -> Huffman {
    let mut lengths = [0u8; 288];
    for (symbol, l) in lengths.iter_mut().enumerate() {
        *l = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Huffman::new(&lengths, "invalid literal/lengths set", false).expect("the fixed code is complete")
}

/// The fixed distance code: 32 five-bit codes, of which 30 and 31 are not valid distances,
/// as zlib's `fixedtables` builds it. [`block`] refuses them when one is decoded.
fn fixed_distances() -> Huffman {
    Huffman::new(&[5u8; 32], "invalid distances set", false).expect("the fixed code is complete")
}

/// The two code tables a dynamic block carries (RFC 1951 section 3.2.7).
fn dynamic_tables(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), GzipError> {
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let nlen = bits.bits(5)? as usize + 257;
    let ndist = bits.bits(5)? as usize + 1;
    let ncode = bits.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(GzipError::zip("too many length or distance symbols"));
    }
    let mut code_lengths = [0u8; 19];
    for &slot in ORDER.iter().take(ncode) {
        code_lengths[slot] = bits.bits(3)? as u8;
    }
    let codes = Huffman::new(&code_lengths, "invalid code lengths set", false)?;
    let mut lengths = vec![0u8; nlen + ndist];
    let mut i = 0;
    while i < lengths.len() {
        let symbol = codes.decode(bits, "invalid code lengths set")?;
        match symbol {
            0..=15 => {
                lengths[i] = symbol as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(GzipError::zip("invalid bit length repeat"));
                }
                let previous = lengths[i - 1];
                let repeat = 3 + bits.bits(2)? as usize;
                i = repeat_length(&mut lengths, i, repeat, previous)?;
            }
            17 => {
                let repeat = 3 + bits.bits(3)? as usize;
                i = repeat_length(&mut lengths, i, repeat, 0)?;
            }
            _ => {
                let repeat = 11 + bits.bits(7)? as usize;
                i = repeat_length(&mut lengths, i, repeat, 0)?;
            }
        }
    }
    if lengths[256] == 0 {
        return Err(GzipError::zip("invalid code -- missing end-of-block"));
    }
    let literals = Huffman::new(&lengths[..nlen], "invalid literal/lengths set", true)?;
    let distances = Huffman::new(&lengths[nlen..], "invalid distances set", true)?;
    Ok((literals, distances))
}

fn repeat_length(lengths: &mut [u8], at: usize, repeat: usize, value: u8) -> Result<usize, GzipError> {
    if at + repeat > lengths.len() {
        return Err(GzipError::zip("invalid bit length repeat"));
    }
    lengths[at..at + repeat].fill(value);
    Ok(at + repeat)
}

/// The literal, length and distance codes of one compressed block.
fn block(bits: &mut Bits<'_>, out: &mut Vec<u8>, lengths: &Huffman, distances: &Huffman) -> Result<(), GzipError> {
    const LENGTH_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227,
        258,
    ];
    const LENGTH_EXTRA: [u32; 29] =
        [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
    const DISTANCE_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097,
        6145, 8193, 12289, 16385, 24577,
    ];
    const DISTANCE_EXTRA: [u32; 30] =
        [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
    loop {
        let symbol = lengths.decode(bits, "invalid literal/length code")?;
        match symbol {
            0..=255 => out.push(symbol as u8),
            256 => return Ok(()),
            _ => {
                let i = usize::from(symbol) - 257;
                if i >= LENGTH_BASE.len() {
                    return Err(GzipError::zip("invalid literal/length code"));
                }
                let length = usize::from(LENGTH_BASE[i]) + bits.bits(LENGTH_EXTRA[i])? as usize;
                let d = usize::from(distances.decode(bits, "invalid distance code")?);
                if d >= DISTANCE_BASE.len() {
                    return Err(GzipError::zip("invalid distance code"));
                }
                let distance = usize::from(DISTANCE_BASE[d]) + bits.bits(DISTANCE_EXTRA[d])? as usize;
                if distance > out.len() {
                    return Err(GzipError::zip("invalid distance too far back"));
                }
                for _ in 0..length {
                    out.push(out[out.len() - distance]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// java.util.zip.CRC32 of "123456789" is cbf43926; of "" is 0.
    #[test]
    fn crc32_matches_java() {
        assert_eq!(format!("{:x}", crc32(b"123456789")), "cbf43926");
        assert_eq!(crc32(b""), 0);
    }

    /// The stored blocks [`gzip`] writes are read back by the same inflater, including the
    /// empty payload and one longer than a single block.
    #[test]
    fn stored_blocks_round_trip() {
        for size in [0usize, 1, 0xffff, 0x10000, 0x20001] {
            let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            assert_eq!(gunzip(&gzip(&data)), Ok(data), "{size} bytes");
        }
    }
}
