//! The CBOR subset ADI-P 10.2 uses, written to match `sdk/c/src/adip_cbor.c`
//! byte for byte.
//!
//! Hand written rather than taken from a crate, for one reason: the appliance
//! side is 341 lines of C that cannot take a dependency, and the two must agree
//! exactly. Tag 4's `[exponent, mantissa]` ordering, the refusal of binary
//! floats, and the head encoding are all things a general purpose library would
//! put behind an abstraction where a disagreement would be invisible.

use crate::num::Num;
use crate::WireError;
use adi_protocol::units::Unit;

pub const MT_UINT: u8 = 0;
pub const MT_NINT: u8 = 1;
pub const MT_TEXT: u8 = 3;
pub const MT_ARRAY: u8 = 4;
pub const MT_MAP: u8 = 5;
pub const MT_TAG: u8 = 6;
pub const MT_SIMPLE: u8 = 7;

/// RFC 8949 tag 4: a decimal fraction, `[exponent, mantissa]`.
pub const TAG_DECIMAL: u64 = 4;

// ------------------------------------------------------------------ writing

#[derive(Default)]
pub struct Writer {
    pub out: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { out: Vec::new() }
    }

    pub fn head(&mut self, mt: u8, value: u64) {
        // The shortest form that holds the value, matching the C exactly. A
        // longer encoding would still be legal CBOR and would produce
        // different bytes for the same message.
        if value < 24 {
            self.out.push((mt << 5) | (value as u8));
        } else if value <= 0xff {
            self.out.push((mt << 5) | 24);
            self.out.push(value as u8);
        } else if value <= 0xffff {
            self.out.push((mt << 5) | 25);
            self.out.extend_from_slice(&(value as u16).to_be_bytes());
        } else if value <= 0xffff_ffff {
            self.out.push((mt << 5) | 26);
            self.out.extend_from_slice(&(value as u32).to_be_bytes());
        } else {
            self.out.push((mt << 5) | 27);
            self.out.extend_from_slice(&value.to_be_bytes());
        }
    }

    pub fn uint(&mut self, v: u64) {
        self.head(MT_UINT, v);
    }

    pub fn int(&mut self, v: i64) {
        if v < 0 {
            // CBOR negative integers encode -1 - n.
            self.head(MT_NINT, (-1 - v) as u64);
        } else {
            self.head(MT_UINT, v as u64);
        }
    }

    pub fn bool(&mut self, v: bool) {
        // Simple values: 20 is false, 21 is true.
        self.head(MT_SIMPLE, if v { 21 } else { 20 });
    }

    pub fn text(&mut self, s: &str) {
        self.head(MT_TEXT, s.len() as u64);
        self.out.extend_from_slice(s.as_bytes());
    }

    pub fn map(&mut self, count: u64) {
        self.head(MT_MAP, count);
    }

    pub fn array(&mut self, count: u64) {
        self.head(MT_ARRAY, count);
    }

    /// A bare magnitude: tag 4 over `[exponent, mantissa]`.
    pub fn num(&mut self, n: Num) {
        self.head(MT_TAG, TAG_DECIMAL);
        self.array(2);
        self.int(i64::from(n.exponent));
        self.int(i64::from(n.mantissa));
    }

    /// A quantity: `[unit, number]`, ADI-P 10.2.3.
    pub fn quantity(&mut self, n: Num) {
        self.array(2);
        self.uint(u64::from(n.unit.as_wire()));
        self.num(n);
    }
}

// ------------------------------------------------------------------ reading

pub struct Reader<'a> {
    buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.pos + n > self.buf.len() {
            return Err(WireError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn head(&mut self) -> Result<(u8, u64), WireError> {
        let first = *self.take(1)?.first().ok_or(WireError::Truncated)?;
        let mt = first >> 5;
        let ai = first & 0x1f;

        let value = match ai {
            0..=23 => u64::from(ai),
            24 => u64::from(self.take(1)?[0]),
            25 => {
                let b = self.take(2)?;
                u64::from(u16::from_be_bytes([b[0], b[1]]))
            }
            26 => {
                let b = self.take(4)?;
                u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            }
            27 => {
                let b = self.take(8)?;
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
            // 28 to 30 are reserved; 31 is the indefinite length form, which
            // this protocol does not use and a decoder must not invent a
            // meaning for.
            _ => return Err(WireError::Malformed("reserved or indefinite length")),
        };

        Ok((mt, value))
    }

    fn expect(&mut self, want: u8) -> Result<u64, WireError> {
        let (mt, value) = self.head()?;
        if mt != want {
            return Err(WireError::Malformed("unexpected major type"));
        }
        Ok(value)
    }

    pub fn uint(&mut self) -> Result<u64, WireError> {
        self.expect(MT_UINT)
    }

    pub fn int(&mut self) -> Result<i64, WireError> {
        let (mt, value) = self.head()?;
        match mt {
            MT_UINT => i64::try_from(value).map_err(|_| WireError::Malformed("integer too wide")),
            MT_NINT => i64::try_from(value)
                .map(|v| -1 - v)
                .map_err(|_| WireError::Malformed("integer too wide")),
            _ => Err(WireError::Malformed("not an integer")),
        }
    }

    pub fn bool(&mut self) -> Result<bool, WireError> {
        let (mt, value) = self.head()?;
        match (mt, value) {
            (MT_SIMPLE, 20) => Ok(false),
            (MT_SIMPLE, 21) => Ok(true),
            _ => Err(WireError::Malformed("not a boolean")),
        }
    }

    pub fn text(&mut self) -> Result<&'a str, WireError> {
        let len = self.expect(MT_TEXT)? as usize;
        let bytes = self.take(len)?;
        core::str::from_utf8(bytes).map_err(|_| WireError::Malformed("text is not utf-8"))
    }

    pub fn map(&mut self) -> Result<u64, WireError> {
        self.expect(MT_MAP)
    }

    pub fn array(&mut self) -> Result<u64, WireError> {
        self.expect(MT_ARRAY)
    }

    /// Read a magnitude. A plain integer is accepted as an exponent of zero.
    ///
    /// A binary float is refused rather than converted: it cannot hold 0.1, and
    /// accepting one would put back the ambiguity the decimal form removes.
    pub fn num(&mut self, unit: Unit) -> Result<Num, WireError> {
        let save = self.pos;
        let (mt, value) = self.head()?;

        if mt == MT_UINT || mt == MT_NINT {
            self.pos = save;
            let v = self.int()?;
            let mantissa =
                i32::try_from(v).map_err(|_| WireError::Malformed("mantissa too wide"))?;
            return Ok(Num::new(mantissa, 0, unit));
        }

        if mt == MT_SIMPLE {
            // 25, 26 and 27 in major type 7 are the half, single and double
            // precision floats.
            return Err(WireError::BinaryFloat);
        }

        if mt != MT_TAG || value != TAG_DECIMAL {
            return Err(WireError::Malformed("not a decimal fraction"));
        }

        if self.array()? != 2 {
            return Err(WireError::Malformed("tag 4 needs exactly two items"));
        }

        let exponent = self.int()?;
        let mantissa = self.int()?;

        Ok(Num::new(
            i32::try_from(mantissa).map_err(|_| WireError::Malformed("mantissa too wide"))?,
            i8::try_from(exponent).map_err(|_| WireError::Malformed("exponent too wide"))?,
            unit,
        ))
    }

    /// Read `[unit, number]`, refusing a unit this version does not define.
    pub fn quantity(&mut self) -> Result<Num, WireError> {
        if self.array()? != 2 {
            return Err(WireError::Malformed("a quantity is [unit, number]"));
        }
        let raw = self.uint()?;
        let unit = u8::try_from(raw)
            .ok()
            .and_then(Unit::from_wire)
            .ok_or(WireError::UnknownUnit(raw))?;
        self.num(unit)
    }

    /// Skip one item, including nested containers.
    ///
    /// Iterative rather than recursive, as in the C, so a deeply nested hostile
    /// input cannot drive it into the stack.
    pub fn skip(&mut self) -> Result<(), WireError> {
        let mut pending: u64 = 1;

        while pending > 0 {
            pending -= 1;
            let (mt, value) = self.head()?;
            match mt {
                MT_UINT | MT_NINT | MT_SIMPLE => {}
                MT_TEXT | 2 => {
                    self.take(value as usize)?;
                }
                MT_ARRAY => pending = pending.saturating_add(value),
                MT_MAP => pending = pending.saturating_add(value.saturating_mul(2)),
                MT_TAG => pending = pending.saturating_add(1),
                _ => return Err(WireError::Malformed("unknown major type")),
            }
        }

        Ok(())
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }
}
