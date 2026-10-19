// SPDX-License-Identifier: Apache-2.0
//! Source encodings (RML-IO `rml:encoding`, CSVW `csvw:encoding`).
//!
//! Every source format reads UTF-8, so other encodings are decoded to UTF-8
//! as a stream: a UTF-16 CSV file is still read row by row.
//!
//! UTF-16 follows RFC 2781: a byte order mark at the start gives the byte
//! order and is dropped; without one, the encoding's default order is used.
//! Unpaired surrogates and a dangling odd byte are errors, never replaced.

use std::io::{self, BufRead, BufReader, Read};

use rmlrs_model::SourceEncoding;

/// Wraps a reader so that reading yields UTF-8.
pub fn decode<'a>(
    reader: Box<dyn BufRead + 'a>,
    encoding: SourceEncoding,
) -> Box<dyn BufRead + 'a> {
    match encoding {
        SourceEncoding::Utf8 => reader,
        SourceEncoding::Utf16Be => Box::new(BufReader::new(Utf16::new(reader, false))),
        SourceEncoding::Utf16Le => Box::new(BufReader::new(Utf16::new(reader, true))),
    }
}

/// Decodes UTF-16 into UTF-8.
struct Utf16<R> {
    inner: R,
    little_endian: bool,
    /// The first code unit (a possible byte order mark) has been seen.
    started: bool,
    /// The first byte of a code unit split across two reads.
    odd: Option<u8>,
    /// A high surrogate waiting for its low half.
    high: Option<u16>,
    raw: Vec<u8>,
    out: Vec<u8>,
    pos: usize,
    eof: bool,
}

impl<R: Read> Utf16<R> {
    fn new(inner: R, little_endian: bool) -> Self {
        Utf16 {
            inner,
            little_endian,
            started: false,
            odd: None,
            high: None,
            raw: vec![0; 1 << 15],
            out: Vec::new(),
            pos: 0,
            eof: false,
        }
    }

    /// Decodes the next chunk of input into `out`.
    fn fill(&mut self) -> io::Result<()> {
        self.out.clear();
        self.pos = 0;
        let n = loop {
            match self.inner.read(&mut self.raw) {
                Ok(n) => break n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        };
        if n == 0 {
            self.eof = true;
            if self.odd.is_some() || self.high.is_some() {
                return Err(invalid("UTF-16 data ends in the middle of a character"));
            }
            return Ok(());
        }
        let raw = std::mem::take(&mut self.raw);
        let result = self.decode_units(&raw[..n]);
        self.raw = raw;
        result
    }

    fn decode_units(&mut self, raw: &[u8]) -> io::Result<()> {
        let mut bytes = raw.iter().copied();
        loop {
            let (a, b) = match self.odd.take() {
                Some(a) => match bytes.next() {
                    Some(b) => (a, b),
                    None => {
                        self.odd = Some(a);
                        break;
                    }
                },
                None => match (bytes.next(), bytes.next()) {
                    (Some(a), Some(b)) => (a, b),
                    (Some(a), None) => {
                        self.odd = Some(a);
                        break;
                    }
                    _ => break,
                },
            };
            let unit = if self.little_endian {
                u16::from_le_bytes([a, b])
            } else {
                u16::from_be_bytes([a, b])
            };
            if !self.started {
                self.started = true;
                match unit {
                    0xFEFF => continue,
                    0xFFFE => {
                        self.little_endian = !self.little_endian;
                        continue;
                    }
                    _ => {}
                }
            }
            self.unit(unit)?;
        }
        Ok(())
    }

    fn unit(&mut self, unit: u16) -> io::Result<()> {
        let c = match (self.high.take(), unit) {
            (None, 0xD800..=0xDBFF) => {
                self.high = Some(unit);
                return Ok(());
            }
            (None, 0xDC00..=0xDFFF) => return Err(invalid("unpaired UTF-16 surrogate")),
            (None, u) => char::from_u32(u32::from(u)),
            (Some(h), 0xDC00..=0xDFFF) => char::from_u32(
                0x10000 + ((u32::from(h) - 0xD800) << 10) + (u32::from(unit) - 0xDC00),
            ),
            (Some(_), _) => return Err(invalid("unpaired UTF-16 surrogate")),
        };
        let c = c.ok_or_else(|| invalid("invalid UTF-16 code unit"))?;
        let mut buf = [0; 4];
        self.out
            .extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        Ok(())
    }
}

impl<R: Read> Read for Utf16<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.out.len() {
            if self.eof {
                return Ok(0);
            }
            self.fill()?;
        }
        let n = buf.len().min(self.out.len() - self.pos);
        buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(bytes: &[u8], encoding: SourceEncoding) -> io::Result<String> {
        let mut out = String::new();
        decode(Box::new(bytes), encoding).read_to_string(&mut out)?;
        Ok(out)
    }

    fn utf16(text: &str, le: bool, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let units = bom.then_some(0xFEFF).into_iter().chain(text.encode_utf16());
        for u in units {
            out.extend(if le { u.to_le_bytes() } else { u.to_be_bytes() });
        }
        out
    }

    const TEXT: &str = "ID,Name\n10,Vénus 𝄞\n";

    #[test]
    fn utf8_is_passed_through() {
        assert_eq!(read(TEXT.as_bytes(), SourceEncoding::Utf8).unwrap(), TEXT);
    }

    #[test]
    fn byte_order_mark_wins_over_the_default_order() {
        for default in [SourceEncoding::Utf16Be, SourceEncoding::Utf16Le] {
            for le in [false, true] {
                assert_eq!(read(&utf16(TEXT, le, true), default).unwrap(), TEXT);
            }
        }
    }

    #[test]
    fn without_a_byte_order_mark_the_default_order_is_used() {
        let be = utf16(TEXT, false, false);
        let le = utf16(TEXT, true, false);
        assert_eq!(read(&be, SourceEncoding::Utf16Be).unwrap(), TEXT);
        assert_eq!(read(&le, SourceEncoding::Utf16Le).unwrap(), TEXT);
        assert_ne!(read(&le, SourceEncoding::Utf16Be).unwrap(), TEXT);
    }

    #[test]
    fn surrogate_pairs_split_across_reads() {
        // Reading one byte at a time splits every code unit and surrogate pair.
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = buf.len().min(self.0.len()).min(1);
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let bytes = utf16(TEXT, true, true);
        let inner = BufReader::with_capacity(1, OneByte(&bytes));
        let mut out = String::new();
        decode(Box::new(inner), SourceEncoding::Utf16Be)
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(out, TEXT);
    }

    #[test]
    fn malformed_utf16_is_an_error() {
        // A lone low surrogate, a lone high surrogate, and an odd byte count.
        assert!(read(&[0xDC, 0x00], SourceEncoding::Utf16Be).is_err());
        assert!(read(&[0xD8, 0x00, 0x00, 0x41], SourceEncoding::Utf16Be).is_err());
        assert!(read(&[0xD8, 0x00], SourceEncoding::Utf16Be).is_err());
        assert!(read(&[0x00, 0x41, 0x00], SourceEncoding::Utf16Be).is_err());
    }
}
