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
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use adi_wire::{decode, encode_framed, frame, Message};

use crate::{SessionError, Transport};

/// An appliance started as a child process.
pub struct ChildAppliance {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
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

        Ok(Self {
            child,
            stdin,
            stdout,
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

    fn recv(&mut self, _timeout_ms: u32) -> Result<Option<Message>, SessionError> {
        if let Some(message) = self.take_buffered() {
            return Some(message).transpose();
        }

        // A blocking read, which is correct for a child process the engine
        // drives: the appliance answers every message, and the engine has
        // nothing else to do until it does. A transport over a real link
        // would honour the timeout, because there a silence is ordinary and
        // the heartbeat still has to go out.
        let mut chunk = [0u8; 1024];
        let read = self
            .stdout
            .read(&mut chunk)
            .map_err(|e| SessionError::Link(format!("could not read from the appliance: {e}")))?;

        if read == 0 {
            return Err(SessionError::Link("the appliance closed the link".into()));
        }

        self.buffer.extend_from_slice(&chunk[..read]);
        self.take_buffered().transpose()
    }
}

impl Drop for ChildAppliance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
