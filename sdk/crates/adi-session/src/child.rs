//! An appliance running as a child process.
//!
//! The local IPC binding of ADI-P 10. The engine starts the appliance's
//! firmware build, writes framed messages to its standard input and reads them
//! from its standard output.
//!
//! This is how `sdk/c/example` is driven, and it is a real transport rather
//! than a testing convenience: a manufacturer developing firmware on a desktop
//! before any board exists uses exactly this. The bytes are identical to the
//! ones a UART would carry.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use adi_wire::{decode, encode_framed, frame, Message};

use crate::{SessionError, Transport};

/// An appliance started as a child process.
pub struct ChildAppliance {
    child: Child,
    stdin: ChildStdin,
    /// Bytes the appliance has written, handed over by the reading thread.
    ///
    /// A read from a child's pipe cannot be given a deadline on either
    /// platform, so the read happens on a thread of its own and the timeout
    /// applies to waiting for what it has read. See `recv`.
    incoming: Receiver<Vec<u8>>,
    /// Bytes received and not yet consumed. A frame can arrive in pieces, and
    /// a read can return several frames at once.
    buffer: Vec<u8>,
}

impl ChildAppliance {
    /// Start the appliance.
    pub fn start(command: &str, args: &[String]) -> Result<Self, SessionError> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherited rather than captured: an appliance writes diagnostics
            // there, and swallowing them during development is unhelpful.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| SessionError::Link(format!("could not start '{command}': {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SessionError::Link("the appliance has no standard input".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SessionError::Link("the appliance has no standard output".into()))?;

        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || {
            let mut stdout = stdout;
            let mut chunk = [0u8; 1024];
            loop {
                match stdout.read(&mut chunk) {
                    // End of output, or a broken pipe once the child is gone.
                    // Either way there is nothing further to read, and
                    // dropping the sender is what tells `recv` so.
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if sender.send(chunk[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            incoming,
            buffer: Vec::new(),
        })
    }

    /// Take a complete frame from the buffer, if one is there.
    fn take_buffered(&mut self) -> Option<Result<Message, SessionError>> {
        match frame::find(&self.buffer) {
            frame::Found::Frame { start, end, consumed } => {
                let decoded = decode(&self.buffer[start..end])
                    .map_err(|e| SessionError::Link(format!("the appliance sent {e}")));
                self.buffer.drain(..consumed);
                Some(decoded)
            }
            frame::Found::Incomplete { .. } => None,
        }
    }
}

impl Transport for ChildAppliance {
    fn send(&mut self, message: &Message) -> Result<(), SessionError> {
        let bytes = encode_framed(message)?;
        self.stdin
            .write_all(&bytes)
            .and_then(|()| self.stdin.flush())
            .map_err(|e| SessionError::Link(format!("could not reach the appliance: {e}")))
    }

    fn recv(&mut self, timeout_ms: u32) -> Result<Option<Message>, SessionError> {
        if let Some(message) = self.take_buffered() {
            return Some(message).transpose();
        }

        // The timeout is honoured, and it has to be. This read used to block
        // until the appliance spoke, on the reasoning that an appliance
        // answers every message and the engine has nothing else to do until
        // it does. That holds only while the appliance speaks when spoken to.
        // An appliance reports its own state when it changes, so the engine
        // reads until the link is quiet, and a read with no end to it turns
        // the end of that drain into a deadlock.
        //
        // A silence is an ordinary answer here, as it is over a serial link,
        // and the caller decides what it means.
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));

        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.incoming.recv_timeout(left) {
                Ok(chunk) => {
                    self.buffer.extend_from_slice(&chunk);
                    if let Some(message) = self.take_buffered() {
                        return Some(message).transpose();
                    }
                    // A frame in pieces. Keep waiting for the rest of it,
                    // within the same deadline.
                }
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(SessionError::Link("the appliance closed the link".into()))
                }
            }
        }
    }
}

impl Drop for ChildAppliance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
