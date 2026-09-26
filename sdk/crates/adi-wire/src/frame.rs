//! Framing for stream transports. ADI-P 10.2.1.
//!
//! ```text
//!   0xAD   len_hi   len_lo   <payload ...>   crc_hi   crc_lo
//! ```
//!
//! Matches `sdk/c/src/adip_frame.c`. The sync byte lets a receiver
//! resynchronise after corruption, the length is checked before any payload is
//! buffered, and the CRC covers the payload only.

use crate::WireError;

pub const SYNC: u8 = 0xAD;
pub const HEADER: usize = 3;
pub const TRAILER: usize = 2;

/// Largest payload a frame may carry, as the appliance enforces it.
pub const MAX_PAYLOAD: usize = 512;

/// CRC-16/CCITT-FALSE: polynomial 0x1021, initial value 0xFFFF, no reflection
/// and no final xor.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Wrap a payload for a stream transport.
pub fn wrap(payload: &[u8]) -> Result<Vec<u8>, WireError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(WireError::TooLong(payload.len()));
    }

    let mut out = Vec::with_capacity(HEADER + payload.len() + TRAILER);
    out.push(SYNC);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&crc16(payload).to_be_bytes());
    Ok(out)
}

/// What a scan of a receive buffer found.
pub enum Found {
    /// A complete frame. `payload` is the range within the input, and
    /// `consumed` is how much of the buffer may now be discarded.
    Frame { start: usize, end: usize, consumed: usize },
    /// Nothing wrong, just not all here yet. `consumed` is what may be
    /// discarded regardless, which is how a receiver makes progress past
    /// leading rubbish.
    Incomplete { consumed: usize },
}

/// Locate one complete frame.
///
/// A bad CRC or a length past the maximum is not an error the caller can fix,
/// so the frame is skipped and the scan continues. That is what lets a receiver
/// resynchronise on a noisy line instead of wedging on the first corrupt byte.
pub fn find(buf: &[u8]) -> Found {
    let mut i = 0;

    while i < buf.len() {
        if buf[i] != SYNC {
            i += 1;
            continue;
        }

        if i + HEADER > buf.len() {
            return Found::Incomplete { consumed: i };
        }

        let len = usize::from(u16::from_be_bytes([buf[i + 1], buf[i + 2]]));

        if len > MAX_PAYLOAD {
            // Not a frame we could ever accept. Step past this sync byte and
            // keep looking rather than trusting the length.
            i += 1;
            continue;
        }

        let total = HEADER + len + TRAILER;
        if i + total > buf.len() {
            return Found::Incomplete { consumed: i };
        }

        let start = i + HEADER;
        let end = start + len;
        let want = u16::from_be_bytes([buf[end], buf[end + 1]]);

        if crc16(&buf[start..end]) == want {
            return Found::Frame { start, end, consumed: i + total };
        }

        // The CRC failed, so this was noise that happened to start with the
        // sync byte. Resynchronise past it.
        i += 1;
    }

    Found::Incomplete { consumed: buf.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check value every CRC-16/CCITT-FALSE implementation agrees on, so a
    /// disagreement with the C shows up here rather than on a wire.
    #[test]
    fn crc_matches_the_standard_check_vector() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn a_wrapped_payload_is_found_again() {
        let framed = wrap(b"hello").unwrap();
        match find(&framed) {
            Found::Frame { start, end, consumed } => {
                assert_eq!(&framed[start..end], b"hello");
                assert_eq!(consumed, framed.len());
            }
            _ => panic!("should have found the frame"),
        }
    }

    #[test]
    fn leading_rubbish_is_skipped() {
        let mut buf = vec![0x00, 0xff, 0x12];
        buf.extend_from_slice(&wrap(b"payload").unwrap());
        match find(&buf) {
            Found::Frame { start, end, .. } => assert_eq!(&buf[start..end], b"payload"),
            _ => panic!("should have resynchronised"),
        }
    }

    #[test]
    fn a_partial_frame_waits() {
        let framed = wrap(b"hello").unwrap();
        assert!(matches!(
            find(&framed[..framed.len() - 1]),
            Found::Incomplete { .. }
        ));
    }

    /// A corrupt frame must not wedge the receiver: the next good one still
    /// has to be found.
    #[test]
    fn a_bad_crc_resynchronises_to_the_next_frame() {
        let mut buf = wrap(b"first").unwrap();
        let last = buf.len() - 1;
        buf[last] ^= 0xff;
        buf.extend_from_slice(&wrap(b"second").unwrap());

        match find(&buf) {
            Found::Frame { start, end, .. } => assert_eq!(&buf[start..end], b"second"),
            _ => panic!("should have found the second frame"),
        }
    }

    #[test]
    fn an_oversized_payload_is_refused() {
        assert!(matches!(
            wrap(&vec![0u8; MAX_PAYLOAD + 1]),
            Err(WireError::TooLong(_))
        ));
    }
}
