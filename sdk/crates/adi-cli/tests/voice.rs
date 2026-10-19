//! The voice seams, as the tool actually wires them.
//!
//! `speech_backend.rs` proves that audio reaches a program and words come
//! back. It does not prove that the tool hands that program what the
//! configuration said, and it did not: the sample rate was hardcoded at the
//! call site and the confidence threshold was never read at all. Both fields
//! were present in the shipped example configurations and documented on the
//! type that holds them, and the threshold's default happened to equal the
//! value in those examples, so the omission was invisible from the outside.
//!
//! These drive the built binary against the C example appliance, because the
//! wiring is what was wrong and the wiring exists only in the binary.
//!
//! Skipped rather than failed where Python or the example appliance is
//! absent. The appliance is built with `cd c && make example`.

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

/// The appliance, under whichever name this platform built it.
fn appliance() -> Option<&'static str> {
    ["c/build/example", "c/build/example.exe"]
        .into_iter()
        .find(|name| sdk().join(name).exists())
}

/// A 16-bit mono WAV, written where a test can point the tool at it.
fn wav(name: &str, amplitude: i16, rate: u32, seconds: f64) -> PathBuf {
    let count = (f64::from(rate) * seconds) as usize;
    let samples: Vec<i16> = (0..count)
        .map(|n| {
            let t = n as f64 / f64::from(rate);
            (f64::from(amplitude) * (2.0 * std::f64::consts::PI * 180.0 * t).sin()) as i16
        })
        .collect();

    let mut out = Vec::new();
    let data = samples.len() * 2;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    for sample in &samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }

    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, out).expect("the wav should be written");
    path
}

/// A models configuration naming the reference speech programs.
fn config(name: &str, python: &str, rate: Option<u32>, threshold: Option<f64>) -> PathBuf {
    let mut text = String::new();
    text.push_str("root: .\n");
    text.push_str("seams:\n");
    text.push_str("  interpreter:\n");
    text.push_str("    backend: process\n");
    text.push_str(&format!("    command: {python}\n"));
    text.push_str("    args: [\"examples/interpreters/reference.py\"]\n");
    text.push_str("  transcriber:\n");
    text.push_str("    backend: process\n");
    text.push_str(&format!("    command: {python}\n"));
    text.push_str("    args: [\"examples/interpreters/reference_speech.py\", \"transcribe\"]\n");
    if let Some(rate) = rate {
        text.push_str(&format!("    sample_rate: {rate}\n"));
    }
    if let Some(threshold) = threshold {
        text.push_str(&format!("    min_confidence: {threshold}\n"));
    }
    text.push_str("  speaker:\n");
    text.push_str("    backend: process\n");
    text.push_str(&format!("    command: {python}\n"));
    text.push_str("    args: [\"examples/interpreters/reference_speech.py\", \"speak\"]\n");

    write_config(name, &text)
}

fn write_config(name: &str, text: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, text).expect("the config should be written");
    path
}

/// Open a session, type the lines, and return everything it printed.
fn session(config: &Path, appliance: &str, lines: &[String]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_adi"))
        .args(["run", "--config"])
        .arg(config)
        .args(["--appliance", appliance])
        .current_dir(sdk())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tool should start");

    {
        let stdin = child.stdin.as_mut().unwrap();
        for line in lines {
            writeln!(stdin, "{line}").unwrap();
        }
        writeln!(stdin, "quit").unwrap();
    }

    let out = child.wait_with_output().expect("should finish");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn listen(path: &Path) -> Vec<String> {
    vec![format!("listen {}", path.display())]
}

macro_rules! setup {
    () => {
        match (python(), appliance()) {
            (Some(python), Some(appliance)) => (python, appliance),
            (None, _) => {
                eprintln!("skipping: no python on PATH");
                return;
            }
            (_, None) => {
                eprintln!("skipping: no example appliance; build it with `cd c && make example`");
                return;
            }
        }
    };
}

/// The defect this file exists for.
///
/// The reference transcriber scores a near-silent recording at about 0.01 and
/// a loud one near 0.99, so a threshold between them decides whether the
/// machine acts or asks. Before the fix the configured number was discarded
/// and the library default of 0.6 was used whatever the file said, which put
/// both assertions below on the wrong side.
#[test]
fn the_configured_threshold_decides_whether_the_machine_asks() {
    let (python, appliance) = setup!();

    let permissive = session(
        &config("permissive.yaml", python, None, Some(0.001)),
        appliance,
        &listen(&wav("quiet.wav", 150, 16_000, 1.0)),
    );
    assert!(
        permissive.contains("understood"),
        "a threshold of 0.001 should accept a transcript scored near 0.01:\n{permissive}"
    );

    let strict = session(
        &config("strict.yaml", python, None, Some(0.995)),
        appliance,
        &listen(&wav("loud.wav", 20_000, 16_000, 1.0)),
    );
    assert!(
        strict.contains("asks"),
        "a threshold of 0.995 should refuse a transcript scored below it:\n{strict}"
    );
}

/// The threshold that was applied has to be the one that was configured, and
/// visibly so: an integrator tuning this number needs to see that it took.
#[test]
fn the_threshold_in_use_is_reported() {
    let (python, appliance) = setup!();
    let out = session(
        &config("reported.yaml", python, None, Some(0.85)),
        appliance,
        &[],
    );
    assert!(
        out.contains("asking below 0.85"),
        "the banner should state the threshold in force:\n{out}"
    );
}

/// ADI-P 9.4.2 refuses audio at a rate other than the model's rather than
/// resampling it, so the rate the tool passes to the backend decides whether
/// that check protects anything. Before the fix it was hardcoded, and a
/// configuration naming any other rate was accepted and ignored.
#[test]
fn the_configured_sample_rate_reaches_the_transcriber() {
    let (python, appliance) = setup!();
    let out = session(
        &config("rate.yaml", python, Some(8_000), None),
        appliance,
        &listen(&wav("at16k.wav", 20_000, 16_000, 0.5)),
    );

    assert!(
        out.contains("8000 Hz"),
        "a backend declared at 8000 Hz should refuse 16000 Hz audio:\n{out}"
    );
}

/// Piper publishes voices at 16 and at 22.05 kHz, so a tool that reads only
/// its own default can drive half of them.
#[test]
fn a_speaker_rate_other_than_the_default_is_honoured() {
    let (python, appliance) = setup!();
    let mut text =
        std::fs::read_to_string(config("base-for-speaker.yaml", python, None, None)).unwrap();
    text.push_str("    sample_rate: 16000\n");
    let out = session(&write_config("speaker-rate.yaml", &text), appliance, &[]);

    // The speaker's own line, not any line mentioning that rate: the
    // transcriber runs at 16000 Hz by default, so a search over the whole
    // output would pass whatever the speaker did.
    let line = out
        .lines()
        .find(|l| l.trim_start().starts_with("speaker "))
        .unwrap_or_else(|| panic!("no speaker line:\n{out}"));
    assert!(
        line.contains("16000 Hz"),
        "the speaker should start at the configured rate, got: {line}"
    );
}

/// A speech program that will not start used to end the session before the
/// prompt, which also took away `cycle`, the one command that needs no model
/// at all. `models.yaml` states the rule it broke: an engine refuses the
/// operations an unserved seam provides and names the seam, and does not fail.
#[test]
fn a_speech_program_that_cannot_start_leaves_the_rest_working() {
    let (python, appliance) = setup!();

    let mut text = String::new();
    text.push_str("root: .\n");
    text.push_str("seams:\n");
    text.push_str("  interpreter:\n");
    text.push_str("    backend: process\n");
    text.push_str(&format!("    command: {python}\n"));
    text.push_str("    args: [\"examples/interpreters/reference.py\"]\n");
    text.push_str("  transcriber:\n");
    text.push_str("    backend: process\n");
    text.push_str("    command: no-such-speech-program\n");
    text.push_str("    args: []\n");
    text.push_str("  speaker:\n");
    text.push_str("    backend: process\n");
    text.push_str("    command: no-such-speech-program\n");
    text.push_str("    args: []\n");

    let out = session(
        &write_config("broken-speech.yaml", &text),
        appliance,
        &["cycle".to_string()],
    );

    assert!(
        out.contains("transcriber  unavailable"),
        "the seam should be named as unserved:\n{out}"
    );
    assert!(
        out.contains("speaker      unavailable"),
        "and so should the other one:\n{out}"
    );
    assert!(
        out.contains("accepted     by the appliance"),
        "a cycle needs no model and must still run:\n{out}"
    );
}

/// The seam that is missing says so when it is used, rather than being
/// silently absent.
#[test]
fn listening_without_a_transcriber_is_explained() {
    let (python, appliance) = setup!();

    let mut text = String::new();
    text.push_str("root: .\n");
    text.push_str("seams:\n");
    text.push_str("  interpreter:\n");
    text.push_str("    backend: process\n");
    text.push_str(&format!("    command: {python}\n"));
    text.push_str("    args: [\"examples/interpreters/reference.py\"]\n");
    text.push_str("  transcriber:\n");
    text.push_str("    backend: none\n");

    let out = session(
        &write_config("no-transcriber.yaml", &text),
        appliance,
        &listen(&wav("unused.wav", 20_000, 16_000, 0.5)),
    );
    assert!(
        out.contains("no transcriber is configured"),
        "the tool should say which seam is missing:\n{out}"
    );
}
