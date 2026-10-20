//! An appliance on the other end of a serial port.
//!
//! The UART binding of ADI-P 10, which the table there names for the case this
//! serves: the module as a separate board from the appliance.
//!
//! Until this existed the engine could only reach an appliance it started
//! itself, as a child process over standard input and output. That is a real
//! binding and it is the right one while the firmware is a host build, but a
//! machine is not a program the engine can launch. It is already running on
//! the other end of a cable. So the first time anyone moved from a desk to
//! hardware, the SDK's own tooling stopped working and they had to write a
//! byte shuffler before they could test anything.
//!
//! This lives in the command line tool rather than in `adi-session` on
//! purpose. The engine crates compile for `wasm32-unknown-unknown`, which is
//! what lets the whole reasoning path run in a browser, and a serial
//! dependency in `adi-session` would end that. Anything that touches real
//! hardware belongs on this side of the boundary.
//!
//! Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.

use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

use adi_session::{SessionError, Transport};
use adi_wire::{decode, encode_framed, frame, Message};

/// Bytes in, messages out.
///
/// A serial port hands over whatever has arrived, so a frame routinely arrives
/// in pieces and several can arrive at once. Reassembly is the receiver's job.
///
/// Separated from the port so it can be tested without one. This is the part
/// that is easy to get wrong and impossible to check by hand: a frame split
/// across two reads, two frames in one read, and rubbish on the line before a
/// good frame are all ordinary on a real cable and none of them can be
/// reproduced reliably by plugging something in.
#[derive(Default)]
struct Reassembler {
    buffer: Vec<u8>,
}

impl Reassembler {
    fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Take one complete frame from what has arrived so far.
    fn take(&mut self) -> Option<Result<Message, SessionError>> {
        match frame::find(&self.buffer) {
            frame::Found::Frame { start, end, consumed } => {
                let decoded = decode(&self.buffer[start..end])
                    .map_err(|e| SessionError::Link(format!("the appliance sent {e}")));
                self.buffer.drain(..consumed);
                Some(decoded)
            }
            // `consumed` is what may be discarded even with no frame yet,
            // which is how a receiver makes progress past leading rubbish
            // instead of rescanning it on every read.
            frame::Found::Incomplete { consumed } => {
                self.buffer.drain(..consumed);
                None
            }
        }
    }
}

/// An appliance reached over a serial port.
pub struct SerialAppliance {
    port: Box<dyn serialport::SerialPort>,
    incoming: Reassembler,
}

impl SerialAppliance {
    /// Open a port and treat whatever is on it as an appliance.
    ///
    /// Nothing is sent here and nothing is expected back. A port that opens is
    /// not evidence of a machine, and the first heartbeat is what settles that.
    pub fn open(path: &str, baud: u32) -> Result<Self, SessionError> {
        let port = serialport::new(path, baud)
            // Replaced per call in `recv`. A port with no timeout blocks
            // forever, which would hold the heartbeat that detects a dead
            // link.
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| {
                // Three things account for almost every failure here, and the
                // underlying message names none of them.
                SessionError::Link(format!(
                    "could not open {path} at {baud} baud: {e}\n\n\
                     Check the port name, that nothing else holds it open \
                     (a serial monitor will), and on Linux that you are in \
                     the dialout group."
                ))
            })?;

        Ok(Self {
            port,
            incoming: Reassembler::default(),
        })
    }
}

impl Transport for SerialAppliance {
    fn send(&mut self, message: &Message) -> Result<(), SessionError> {
        let bytes = encode_framed(message)?;
        self.port
            .write_all(&bytes)
            // Without this the appliance waits for a message sitting in a
            // driver buffer and the engine waits for the reply. The same
            // mistake on the appliance side is a transmitter that was never
            // started.
            .and_then(|()| self.port.flush())
            .map_err(|e| SessionError::Link(format!("could not reach the appliance: {e}")))
    }

    fn recv(&mut self, timeout_ms: u32) -> Result<Option<Message>, SessionError> {
        if let Some(message) = self.incoming.take() {
            return Some(message).transpose();
        }

        // The timeout is honoured here, unlike the child process binding where
        // a blocking read is correct. On a real link a silence is ordinary:
        // the appliance is quiet for most of a long operation, and the engine
        // still has a heartbeat to send. Blocking would make the thing that
        // detects a dead link the thing that waits forever on one.
        //
        // It bounds the wait for a whole frame, not for one read. A UART
        // delivers whatever has arrived, so a frame routinely comes in two
        // or three pieces: a twelve byte response was seen arriving as nine
        // bytes and then three. Returning after the first read handed the
        // caller None, which says "nothing arrived" and is the one thing it
        // did not mean. The session reads None as silence and gives up, so
        // an appliance that had answered correctly was reported as having
        // gone quiet, and every invocation over a serial link failed that
        // way.
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));

        loop {
            if let Some(message) = self.incoming.take() {
                return Some(message).transpose();
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }

            self.port.set_timeout(remaining).map_err(|e| {
                SessionError::Link(format!("could not set the read timeout: {e}"))
            })?;

            let mut chunk = [0u8; 256];
            match self.port.read(&mut chunk) {
                // A read of nothing is not a frame and not an end of file;
                // the deadline above is what stops this going round forever.
                Ok(0) => continue,
                Ok(read) => self.incoming.push(&chunk[..read]),
                // Nothing arrived in time. Not an error, and not a closed
                // link: a serial port has no end of file, so quiet and gone
                // look identical here. Reporting silence lets the session's
                // own liveness rule decide, which is where that decision
                // belongs.
                Err(e) if e.kind() == ErrorKind::TimedOut => return Ok(None),
                Err(e) => {
                    return Err(SessionError::Link(format!(
                        "could not read from the appliance: {e}"
                    )))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adi_wire::{Num, Outcome, Sample};

    fn reply(id: u32) -> Message {
        Message::Response {
            id,
            outcome: Outcome::Completed,
            reason: None,
        }
    }

    fn framed(message: &Message) -> Vec<u8> {
        encode_framed(message).expect("a reply encodes")
    }

    fn id_of(message: &Message) -> u32 {
        match message {
            Message::Response { id, .. } => *id,
            other => panic!("expected a response, got {other:?}"),
        }
    }

    /// The ordinary case, so the rest has a baseline.
    #[test]
    fn one_frame_in_one_read() {
        let mut r = Reassembler::default();
        r.push(&framed(&reply(7)));
        assert_eq!(id_of(&r.take().unwrap().unwrap()), 7);
        assert!(r.take().is_none(), "and nothing is left over");
    }

    /// A serial port hands over whatever has arrived, which on a real cable is
    /// routinely part of a frame. A receiver that treated a short read as a
    /// short frame would reject every message on a busy line.
    #[test]
    fn a_frame_split_across_reads_is_reassembled() {
        let bytes = framed(&reply(11));
        let mut r = Reassembler::default();

        // One byte at a time, which is the worst case and what a slow link
        // actually does.
        for (n, byte) in bytes.iter().enumerate() {
            r.push(&[*byte]);
            if n + 1 < bytes.len() {
                assert!(
                    r.take().is_none(),
                    "no message may be produced from {} of {} bytes",
                    n + 1,
                    bytes.len()
                );
            }
        }

        assert_eq!(id_of(&r.take().unwrap().unwrap()), 11);
    }

    /// Several frames arrive in one read whenever the appliance answers while
    /// publishing. Taking only the first and discarding the buffer would lose
    /// the rest silently, which is the kind of fault that looks like a flaky
    /// cable.
    #[test]
    fn several_frames_in_one_read_are_all_delivered() {
        let mut bytes = Vec::new();
        for id in 1..=3u32 {
            bytes.extend_from_slice(&framed(&reply(id)));
        }

        let mut r = Reassembler::default();
        r.push(&bytes);

        let got: Vec<u32> = (0..3)
            .map(|_| id_of(&r.take().expect("a frame").expect("decodes")))
            .collect();
        assert_eq!(got, vec![1, 2, 3]);
        assert!(r.take().is_none());
    }

    /// ADI-P 10.2.1: the sync byte exists so a receiver can resynchronise
    /// after corruption. A line is noisy at power-on and the first bytes a
    /// receiver sees are often not a frame at all.
    #[test]
    fn rubbish_before_a_frame_is_skipped() {
        let mut bytes = vec![0x00, 0xFF, 0x13, 0x42];
        bytes.extend_from_slice(&framed(&reply(5)));

        let mut r = Reassembler::default();
        r.push(&bytes);
        assert_eq!(id_of(&r.take().unwrap().unwrap()), 5);
    }

    /// A corrupt frame must not wedge the receiver. The next good frame has to
    /// arrive, or one bad byte ends the session.
    #[test]
    fn a_corrupt_frame_does_not_block_the_next_one() {
        let mut bad = framed(&reply(1));
        let last = bad.len() - 1;
        bad[last] ^= 0xFF; // break the CRC

        let mut bytes = bad;
        bytes.extend_from_slice(&framed(&reply(2)));

        let mut r = Reassembler::default();
        r.push(&bytes);

        // The broken one is skipped rather than reported: a bad CRC is not
        // something the caller can act on, and resynchronising is the point.
        assert_eq!(id_of(&r.take().unwrap().unwrap()), 2);
    }

    /// Nothing is produced from nothing, and an empty read is not an error.
    /// On a serial port silence is ordinary, and conflating it with a closed
    /// link would make the heartbeat fire against a healthy machine.
    #[test]
    fn silence_produces_no_message() {
        let mut r = Reassembler::default();
        assert!(r.take().is_none());
        r.push(&[]);
        assert!(r.take().is_none());
    }

    /// Signals batch, so the largest thing on the wire is a sample frame
    /// rather than a reply. ADI-P 10.2.1 caps a payload at 512 bytes and the
    /// buffer must carry one whole.
    #[test]
    fn a_full_signal_batch_survives_a_byte_at_a_time() {
        let samples: Vec<Sample> = (0..8)
            .map(|n: i16| Sample {
                signal: 0,
                t_ms: u32::from(n as u16) * 50,
                value: Num {
                    mantissa: i32::from(n),
                    exponent: 0,
                    unit: adi_protocol::units::Unit::G,
                },
            })
            .collect();
        let bytes = framed(&Message::Signals(samples));

        let mut r = Reassembler::default();
        for byte in &bytes {
            r.push(&[*byte]);
        }
        assert!(
            matches!(r.take(), Some(Ok(Message::Signals(_)))),
            "a batched signal frame must reassemble"
        );
    }
}
