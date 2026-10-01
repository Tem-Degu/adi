//! Speech and voice, served by an external program.
//!
//! The same arrangement as [`crate::process`]: a program reads one JSON
//! request per line and writes one JSON answer, and needs no dependency on
//! this SDK. In-process backends for ONNX Runtime are a separate matter and
//! are not implemented.
//!
//! ```text
//! transcriber  -> {"audio":"<base64>","sample_rate":16000}
//!              <- {"text":"half of last time","confidence":0.91}
//!
//! speaker      -> {"text":"I have made it finer."}
//!              <- {"audio":"<base64>","sample_rate":22050}
//! ```
//!
//! Audio is base64 because a line protocol cannot carry an arbitrary byte.
//! Samples are 16-bit signed, little endian, single channel, which is what
//! `Audio` holds and what the speech models in this space expect.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use adi_language::{Audio, LanguageError, Speaker, Transcriber, Transcript};
use serde::{Deserialize, Serialize};

use crate::ModelError;

/// A line-oriented child process.
///
/// The interpreter in [`crate::process`] keeps its own copy of this, written
/// first. Two copies is one too many and they should be merged; three would be
/// three places to fix the same deadlock once and miss it twice.
struct Pipe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

struct Program {
    inner: Mutex<Pipe>,
}

impl Program {
    fn start(command: &str, args: &[String]) -> Result<Self, ModelError> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| ModelError::Backend(format!("could not start '{command}': {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ModelError::Backend("the program has no standard input".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ModelError::Backend("the program has no standard output".into()))?;

        Ok(Self {
            inner: Mutex::new(Pipe {
                child,
                stdin,
                stdout: BufReader::new(stdout),
            }),
        })
    }

    fn ask<T: serde::de::DeserializeOwned>(&self, request: &impl Serialize) -> Result<T, ModelError> {
        let mut line = serde_json::to_string(request)
            .map_err(|e| ModelError::Backend(format!("could not encode the request: {e}")))?;
        line.push('\n');

        let mut pipe = self
            .inner
            .lock()
            .map_err(|_| ModelError::Backend("the backend lock was poisoned".into()))?;

        pipe.stdin
            .write_all(line.as_bytes())
            .and_then(|()| pipe.stdin.flush())
            .map_err(|e| ModelError::Backend(format!("could not reach the program: {e}")))?;

        let mut reply = String::new();
        if pipe
            .stdout
            .read_line(&mut reply)
            .map_err(|e| ModelError::Backend(format!("could not read the reply: {e}")))?
            == 0
        {
            return Err(ModelError::Backend("the program closed its output".into()));
        }

        serde_json::from_str(reply.trim()).map_err(|e| {
            ModelError::Backend(format!("the reply is not valid: {e}; got {}", reply.trim()))
        })
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        if let Ok(mut pipe) = self.inner.lock() {
            let _ = pipe.child.kill();
            let _ = pipe.child.wait();
        }
    }
}

// --------------------------------------------------------------------- audio

fn encode_audio(samples: &[i16]) -> String {
    use base64::Engine as _;
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_audio(encoded: &str) -> Result<Vec<i16>, ModelError> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| ModelError::Backend(format!("the audio is not valid base64: {e}")))?;

    if bytes.len() % 2 != 0 {
        return Err(ModelError::Backend(
            "the audio has an odd byte count, so it is not 16-bit samples".into(),
        ));
    }

    Ok(bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect())
}

// ----------------------------------------------------------------- listening

#[derive(Serialize)]
struct TranscribeRequest<'a> {
    audio: &'a str,
    sample_rate: u32,
}

#[derive(Deserialize)]
struct TranscribeReply {
    text: String,
    /// ADI-P 9.4.2 requires one. A backend that cannot report a confidence
    /// cannot serve this seam, whatever its accuracy, because the machine then
    /// has no way to tell a clear instruction from a guess at a muffled one.
    confidence: f64,
}

/// A transcriber that is an external program.
pub struct ProcessTranscriber {
    program: Program,
    sample_rate: u32,
}

impl ProcessTranscriber {
    /// `sample_rate` is what the program's model requires, not what the
    /// microphone produces. Audio at any other rate is refused rather than
    /// resampled, so this must describe the model.
    pub fn start(command: &str, args: &[String], sample_rate: u32) -> Result<Self, ModelError> {
        Ok(Self {
            program: Program::start(command, args)?,
            sample_rate,
        })
    }
}

impl Transcriber for ProcessTranscriber {
    fn required_sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn transcribe(&self, audio: &Audio) -> Result<Transcript, LanguageError> {
        let encoded = encode_audio(&audio.samples);

        let reply: TranscribeReply = self
            .program
            .ask(&TranscribeRequest {
                audio: &encoded,
                sample_rate: audio.sample_rate,
            })
            .map_err(|e| LanguageError::Backend(e.to_string()))?;

        // A confidence outside 0 to 1 cannot be compared with a threshold, and
        // the comparison is what decides whether the machine acts or asks.
        if !(0.0..=1.0).contains(&reply.confidence) {
            return Err(LanguageError::Backend(format!(
                "confidence {} is outside 0 to 1",
                reply.confidence
            )));
        }

        Ok(Transcript::new(reply.text, reply.confidence))
    }
}

// ------------------------------------------------------------------ speaking

#[derive(Serialize)]
struct SpeakRequest<'a> {
    text: &'a str,
}

#[derive(Deserialize)]
struct SpeakReply {
    audio: String,
    sample_rate: u32,
}

/// A speaker that is an external program.
pub struct ProcessSpeaker {
    program: Program,
    sample_rate: u32,
}

impl ProcessSpeaker {
    pub fn start(command: &str, args: &[String], sample_rate: u32) -> Result<Self, ModelError> {
        Ok(Self {
            program: Program::start(command, args)?,
            sample_rate,
        })
    }
}

impl Speaker for ProcessSpeaker {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn speak(&self, text: &str) -> Result<Audio, LanguageError> {
        let reply: SpeakReply = self
            .program
            .ask(&SpeakRequest { text })
            .map_err(|e| LanguageError::Backend(e.to_string()))?;

        let samples =
            decode_audio(&reply.audio).map_err(|e| LanguageError::Backend(e.to_string()))?;

        // Audio returned at a rate other than the one declared would play at
        // the wrong speed and pitch. That is awkward to diagnose by ear and
        // trivial to catch here.
        if reply.sample_rate != self.sample_rate {
            return Err(LanguageError::Backend(format!(
                "the program declared {} Hz and returned {} Hz",
                self.sample_rate, reply.sample_rate
            )));
        }

        Ok(Audio::new(samples, reply.sample_rate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_survives_the_encoding() {
        let samples: Vec<i16> = vec![0, 1, -1, i16::MAX, i16::MIN, 1234, -4321];
        let back = decode_audio(&encode_audio(&samples)).expect("decodes");
        assert_eq!(back, samples, "every sample must come back unchanged");
    }

    #[test]
    fn empty_audio_round_trips() {
        assert_eq!(decode_audio(&encode_audio(&[])).unwrap(), Vec::<i16>::new());
    }

    /// Half a sample is not a sample. Accepting it would drop a byte and shift
    /// every sample after it, which sounds like noise rather than like an
    /// error.
    #[test]
    fn an_odd_byte_count_is_refused() {
        use base64::Engine as _;
        let odd = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
        assert!(decode_audio(&odd).is_err());
    }

    #[test]
    fn rubbish_is_refused_rather_than_decoded() {
        assert!(decode_audio("not base64 at all !!!").is_err());
    }
}
