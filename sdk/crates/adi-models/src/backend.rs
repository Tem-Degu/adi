//! A model that is another program, and the pipe to it.
//!
//! One child process, started once and kept, exchanging one line of JSON per
//! request. Starting a process per request would pay model loading on every
//! utterance, and loading is the expensive part of a language model.
//!
//! **Why this is separate from the seams that use it.** The interpreter and
//! the chooser ask a model two different questions, and a backend tells them
//! apart by the shape of the request: a selection carries candidates and a
//! spoken request does not. Both shipped backends do exactly that, and say so
//! in their own comments. So one program can serve both seams, and before this
//! existed the engine started it twice, because each seam owned its own pipe.
//!
//! Measured on the recommended 4B model, the second process cost about 2.1 GB.
//! Not a second copy of the weights, which `llama.cpp` maps and both processes
//! read from the same physical pages, but a second set of the private buffers
//! that sit beside them: the compute and cache allocations, and tensors
//! repacked into the form the CPU backend wants.
//!
//! One pipe means one mutex, so the two seams take turns. Nothing is lost by
//! that here: the engine interprets an utterance to completion before it
//! chooses anything, so the two are never in flight at the same moment, and
//! within a single request they cannot be, since choosing a mechanism requires
//! knowing the intent first.
//!
//! Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

/// The child and its two pipes, with the name it was started under.
struct Pipe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Kept for error messages. A program that starts and immediately exits
    /// reports only that its output closed, which says nothing about which
    /// program. On Windows `python3` is a store shim that does exactly this.
    command: String,
}

/// A program serving one or more model seams.
///
/// Held behind an `Arc` by each seam that uses it, so the child outlives any
/// one of them and is reaped when the last releases it.
pub struct ProcessBackend {
    inner: Mutex<Pipe>,
}

impl ProcessBackend {
    /// Start the program.
    pub fn start(command: &str, args: &[String]) -> Result<Self, String> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("could not start '{command}': {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "the program has no standard input".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "the program has no standard output".to_string())?;

        Ok(Self {
            inner: Mutex::new(Pipe {
                child,
                stdin,
                stdout: BufReader::new(stdout),
                command: command.to_string(),
            }),
        })
    }

    /// Send one request and read one answer.
    ///
    /// `request` carries no trailing newline; one is added here, because the
    /// line is the frame and a caller forgetting it would hang waiting for a
    /// reply the program has not been asked for yet.
    ///
    /// The error is a plain message rather than a typed one, so that each seam
    /// can report it in the vocabulary its own caller expects.
    pub fn exchange(&self, request: &str) -> Result<String, String> {
        let mut pipe = self
            .inner
            .lock()
            .map_err(|_| "the backend lock was poisoned".to_string())?;

        pipe.stdin
            .write_all(request.as_bytes())
            .and_then(|()| pipe.stdin.write_all(b"\n"))
            .and_then(|()| pipe.stdin.flush())
            .map_err(|e| format!("could not reach the program: {e}"))?;

        let mut reply = String::new();
        if pipe
            .stdout
            .read_line(&mut reply)
            .map_err(|e| format!("could not read the reply: {e}"))?
            == 0
        {
            return Err(format!(
                "'{}' produced no output and exited. Check that the command \
                 exists and that the script path is correct; on Windows, \
                 'python3' is often a store shim that exits immediately, and \
                 'python' is the working command.",
                pipe.command
            ));
        }

        Ok(reply)
    }

    /// The program this was started as, for a label or an error.
    pub fn command(&self) -> String {
        match self.inner.lock() {
            Ok(pipe) => pipe.command.clone(),
            Err(_) => String::from("unknown"),
        }
    }
}

impl Drop for ProcessBackend {
    fn drop(&mut self) {
        if let Ok(mut pipe) = self.inner.lock() {
            let _ = pipe.child.kill();
            let _ = pipe.child.wait();
        }
    }
}
