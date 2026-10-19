//! The speech backends that are shipped with real models.
//!
//! `reference_speech.py` is covered by `speech_backend.rs`, which drives it as
//! a real child process. `speech_backends.py`, the file that runs Moonshine
//! and Piper, had nothing. That is the arrangement that let the D1 and D2 bug
//! ship in the interpreter: the stand-in had eleven test files and the
//! real-weights backend had none, so every test passed while the backend
//! anybody downloading the models would use could not answer at all.
//!
//! These run without weights. `--explain` reports how a request would be
//! served and loads no model, so CI can check the part that is arithmetic.
//! What the models hear and how they sound needs the models.
//!
//! The confidence rule is what these mostly exercise. It is the number that
//! decides whether a machine acts on what it heard or asks again, it is
//! derived rather than reported by Moonshine, and the file says plainly that
//! it is a stand-in awaiting calibration. A stand-in that is checked is worth
//! more than one that is not.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn sdk() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|candidate| {
        Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Ask the backend how it would serve these requests.
fn explain(mode: &str, requests: &[String]) -> Option<Vec<serde_json::Value>> {
    let python = python()?;

    let mut child = Command::new(python)
        .arg(sdk().join("examples/interpreters/speech_backends.py"))
        .args([mode, "--explain"])
        .current_dir(sdk())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the backend should start");

    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }

    let out = child.wait_with_output().expect("should finish");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let plans: Vec<serde_json::Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("the explanation should be JSON"))
        .collect();

    assert_eq!(
        plans.len(),
        requests.len(),
        "{mode} --explain answered {} of {} requests.\nstderr: {}",
        plans.len(),
        requests.len(),
        String::from_utf8_lossy(&out.stderr)
    );

    Some(plans)
}

/// A recording, as the engine puts it on the wire: base64 of 16-bit signed
/// little endian mono samples.
fn recording(amplitude: i16, rate: u32, seconds: f64) -> String {
    use base64::Engine as _;

    let count = (f64::from(rate) * seconds) as usize;
    let mut bytes = Vec::with_capacity(count * 2);
    for n in 0..count {
        let t = n as f64 / f64::from(rate);
        let sample = (f64::from(amplitude) * (2.0 * std::f64::consts::PI * 180.0 * t).sin()) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }

    let audio = base64::engine::general_purpose::STANDARD.encode(bytes);
    serde_json::json!({ "audio": audio, "sample_rate": rate }).to_string()
}

macro_rules! require_python {
    ($e:expr) => {
        match $e {
            Some(value) => value,
            None => {
                eprintln!("skipping: no python on PATH");
                return;
            }
        }
    };
}

#[test]
fn a_transcribe_request_is_recognised_as_one() {
    let plans = require_python!(explain("transcribe", &[recording(20_000, 16_000, 0.5)]));
    assert_eq!(plans[0]["seam"], "transcriber");
}

#[test]
fn a_speak_request_is_recognised_as_one() {
    let plans = require_python!(explain(
        "speak",
        &[serde_json::json!({"text": "I have made it finer."}).to_string()]
    ));
    assert_eq!(plans[0]["seam"], "speaker");
}

/// ADI-P 9.4.2 refuses audio at a rate other than the model's rather than
/// resampling it, so the backend has to state the rate it needs.
#[test]
fn the_rate_the_model_needs_is_reported() {
    let plans = require_python!(explain("transcribe", &[recording(20_000, 16_000, 0.2)]));
    assert_eq!(plans[0]["sample_rate"], 16_000, "Moonshine wants 16 kHz");
}

/// The audio has to be decoded before anything can be said about it. A
/// backend that silently read zero samples from a real recording would score
/// every utterance the same.
#[test]
fn the_audio_is_decoded_rather_than_counted_as_bytes() {
    let plans = require_python!(explain("transcribe", &[recording(20_000, 16_000, 0.5)]));
    assert_eq!(
        plans[0]["samples"], 8_000,
        "half a second at 16 kHz is 8000 samples, got {}",
        plans[0]["samples"]
    );
}

/// The property that matters. Room noise must not reach a usable threshold
/// however many words the model returns for it, because the transcript is the
/// one place where a wrong word is indistinguishable downstream from a right
/// one, and acting on it is a physical action.
#[test]
fn near_silence_cannot_reach_the_threshold_at_any_transcript_length() {
    let plans = require_python!(explain("transcribe", &[recording(120, 16_000, 1.0)]));
    let scores = &plans[0]["confidence"];

    for length in ["nothing_recognised", "one_word", "a_sentence"] {
        let score = scores[length].as_f64().expect("a number");
        assert!(
            score < 0.6,
            "near silence scored {score} at {length}, which the default threshold would accept"
        );
    }
}

/// And the other side of it: a clear recording of a sentence has to get
/// through, or the machine asks again forever and the seam is useless.
#[test]
fn clear_speech_with_a_sentence_passes_the_threshold() {
    let plans = require_python!(explain("transcribe", &[recording(20_000, 16_000, 1.0)]));
    let score = plans[0]["confidence"]["a_sentence"]
        .as_f64()
        .expect("a number");

    assert!(
        score >= 0.6,
        "a clear recording of a sentence scored {score}, below the default threshold"
    );
}

/// A transcript of nothing is not a quiet transcript, it is no transcript.
/// Scoring it above zero would let a model that returned an empty string act.
#[test]
fn a_transcript_with_no_words_scores_zero() {
    let plans = require_python!(explain("transcribe", &[recording(20_000, 16_000, 0.5)]));
    assert_eq!(plans[0]["confidence"]["nothing_recognised"], 0.0);
}

/// A recording with no samples is answered at zero before the rule is
/// reached, so a seam handed an empty buffer asks rather than acts.
#[test]
fn empty_audio_scores_zero_whatever_comes_back() {
    let request = serde_json::json!({"audio": "", "sample_rate": 16_000}).to_string();
    let plans = require_python!(explain("transcribe", &[request]));
    let scores = &plans[0]["confidence"];

    assert_eq!(plans[0]["samples"], 0);
    for length in ["nothing_recognised", "one_word", "a_sentence"] {
        assert_eq!(scores[length], 0.0, "empty audio scored at {length}");
    }
}

/// Louder is not automatically better, but a recording at speech level should
/// not score below one at noise level. An inverted rule would ask on clear
/// speech and act on noise.
#[test]
fn a_louder_recording_never_scores_lower() {
    let plans = require_python!(explain(
        "transcribe",
        &[
            recording(400, 16_000, 1.0),
            recording(6_000, 16_000, 1.0),
            recording(20_000, 16_000, 1.0),
        ]
    ));

    let scores: Vec<f64> = plans
        .iter()
        .map(|p| p["confidence"]["a_sentence"].as_f64().expect("a number"))
        .collect();

    assert!(
        scores[0] <= scores[1] && scores[1] <= scores[2],
        "confidence should not fall as the recording gets clearer: {scores:?}"
    );
}

/// Nothing to say is answered with silence rather than a failure, so a
/// machine that cannot phrase something still has the text.
#[test]
fn empty_text_is_answered_with_silence_rather_than_an_error() {
    let plans = require_python!(explain(
        "speak",
        &[
            serde_json::json!({"text": "   "}).to_string(),
            serde_json::json!({"text": "Done."}).to_string(),
        ]
    ));

    assert_eq!(plans[0]["silence"], true, "blank text is silence");
    assert_eq!(plans[1]["silence"], false, "a sentence is not");
}
