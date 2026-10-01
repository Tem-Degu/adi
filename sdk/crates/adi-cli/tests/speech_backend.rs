//! Speech and voice served by external programs.
//!
//! The interpreter gained this route first and these two did not, which left
//! voice unreachable by any means while the documentation implied otherwise.
//! These tests drive `examples/interpreters/reference_speech.py` as real child
//! processes.
//!
//! The programs contain no model. What is proven here is the path: audio
//! reaches a program, a transcript with a confidence comes back, the
//! confidence decides whether the machine acts or asks, and synthesised audio
//! returns in a form the engine can play.
//!
//! Skipped rather than failed where Python is absent.

use adi_language::{Audio, Speaker, Transcriber};
use adi_models::{ProcessSpeaker, ProcessTranscriber};

const TRANSCRIBER_RATE: u32 = 16_000;
const SPEAKER_RATE: u32 = 22_050;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|candidate| {
        std::process::Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

fn script() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/interpreters/reference_speech.py")
        .to_string_lossy()
        .into_owned()
}

macro_rules! transcriber {
    () => {
        match python() {
            Some(py) => {
                match ProcessTranscriber::start(
                    py,
                    &[script(), "transcribe".into()],
                    TRANSCRIBER_RATE,
                ) {
                    Ok(t) => t,
                    Err(e) => panic!("the program should start: {e}"),
                }
            }
            None => {
                eprintln!("skipping: no python on PATH");
                return;
            }
        }
    };
}

macro_rules! speaker {
    () => {
        match python() {
            Some(py) => {
                match ProcessSpeaker::start(py, &[script(), "speak".into()], SPEAKER_RATE) {
                    Ok(s) => s,
                    Err(e) => panic!("the program should start: {e}"),
                }
            }
            None => {
                eprintln!("skipping: no python on PATH");
                return;
            }
        }
    };
}

/// A tone, standing in for someone speaking.
fn spoken(amplitude: i16, seconds: f64) -> Audio {
    let count = (TRANSCRIBER_RATE as f64 * seconds) as usize;
    let samples = (0..count)
        .map(|n| {
            let t = n as f64 / TRANSCRIBER_RATE as f64;
            (amplitude as f64 * (2.0 * std::f64::consts::PI * 180.0 * t).sin()) as i16
        })
        .collect();
    Audio::new(samples, TRANSCRIBER_RATE)
}

#[test]
fn audio_reaches_a_program_and_words_come_back() {
    let transcriber = transcriber!();

    let transcript = transcriber
        .transcribe(&spoken(20_000, 1.0))
        .expect("the program should answer");

    assert!(!transcript.text.is_empty(), "there are words");
    assert!(
        (0.0..=1.0).contains(&transcript.confidence),
        "confidence {} is outside 0 to 1",
        transcript.confidence
    );
}

/// ADI-P 9.4.2. A transcript the backend is not confident in must be
/// distinguishable from one it is, because mishearing "hotter" as "shorter" is
/// a physical action on a machine with a heating element.
#[test]
fn a_quiet_recording_returns_a_low_confidence() {
    let transcriber = transcriber!();

    let loud = transcriber.transcribe(&spoken(20_000, 1.0)).expect("answers");
    let quiet = transcriber.transcribe(&spoken(200, 1.0)).expect("answers");

    assert!(
        quiet.confidence < loud.confidence,
        "near silence scored {} against {} for a clear recording",
        quiet.confidence,
        loud.confidence
    );
    assert!(
        !quiet.is_reliable(0.6),
        "a recording this quiet should fall below the threshold, got {}",
        quiet.confidence
    );
    assert!(
        loud.is_reliable(0.6),
        "a clear recording should pass, got {}",
        loud.confidence
    );
}

#[test]
fn the_sample_rate_the_program_requires_is_reported() {
    let transcriber = transcriber!();
    assert_eq!(transcriber.required_sample_rate(), TRANSCRIBER_RATE);
}

#[test]
fn text_reaches_a_program_and_audio_comes_back() {
    let speaker = speaker!();

    let audio = speaker
        .speak("I have made it finer.")
        .expect("the program should answer");

    assert!(!audio.is_empty(), "there is sound");
    assert_eq!(audio.sample_rate, SPEAKER_RATE);
    assert!(
        audio.duration_s() > 0.1,
        "a sentence should last longer than {} s",
        audio.duration_s()
    );
    assert!(
        audio.samples.iter().any(|s| *s != 0),
        "the audio is not silence"
    );
}

/// Audio returned at a rate other than the one declared would play at the
/// wrong speed and pitch, which is awkward to diagnose by ear.
#[test]
fn a_longer_sentence_produces_longer_audio() {
    let speaker = speaker!();

    let short = speaker.speak("Done.").expect("answers");
    let long = speaker
        .speak("I made the grind finer because the shot ran short of its target time.")
        .expect("answers");

    assert!(
        long.duration_s() > short.duration_s(),
        "{} s should exceed {} s",
        long.duration_s(),
        short.duration_s()
    );
}

/// Both programs are started once and kept, because loading a model is the
/// expensive part and paying it per utterance would make the seam unusable.
#[test]
fn both_programs_survive_repeated_use() {
    let transcriber = transcriber!();
    let speaker = speaker!();

    for n in 0..4 {
        assert!(
            transcriber.transcribe(&spoken(15_000, 0.5)).is_ok(),
            "transcribe {n} failed"
        );
        assert!(speaker.speak("Again.").is_ok(), "speak {n} failed");
    }
}
