//! The autonomy levels, running against a real appliance.
//!
//! All three selectors have existed and been unit tested since early on. What
//! did not exist was any way to reach D1 or D2 from a session: `Session::open`
//! calls `selector_for`, which can only build D0, so the model-driven levels
//! had never run outside `adi-decision`'s own tests however complete they were.
//!
//! These tests exercise them end to end: a model-backed chooser picks the
//! action, the engine clamps and checks it, and a real `libadip` build accepts
//! the invocation.
//!
//! Skipped where the appliance or Python is absent.

use std::io::Write;
use std::process::{Command, Stdio};

fn appliance_built() -> bool {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../c/build");
    base.join("example.exe").exists() || base.join("example").exists()
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

fn sdk() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A models configuration naming whichever Python this machine has.
///
/// The shipped example says `python3`, which on Windows is a store shim that
/// exits immediately. Written per run rather than committed, so the test does
/// not depend on which name happens to work here.
fn config_for(py: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("adi-autonomy-{}.yaml", std::process::id()));
    std::fs::write(
        &path,
        format!(
            "root: .\nseams:\n  interpreter:\n    backend: process\n    command: {py}\n    args: [\"examples/interpreters/reference.py\"]\n"
        ),
    )
    .unwrap();
    path
}

fn drive(input: &str, args: &[&str]) -> String {
    let mut all = vec!["run"];
    all.extend_from_slice(args);

    let mut child = Command::new(env!("CARGO_BIN_EXE_adi"))
        .args(&all)
        .current_dir(sdk())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the adi binary should start");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();

    let out = child.wait_with_output().expect("should finish");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

macro_rules! setup {
    () => {{
        if !appliance_built() {
            eprintln!("skipping: build it with `cd sdk/c && make example`");
            return;
        }
        match python() {
            Some(py) => config_for(py),
            None => {
                eprintln!("skipping: no python on PATH");
                return;
            }
        }
    }};
}

/// Extract the value sent to the appliance, so levels can be compared.
fn sent_value(out: &str) -> Option<f64> {
    out.lines()
        .find(|l| l.contains("sent "))
        .and_then(|l| l.split('=').nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

/// ADI-P 3.3 makes D2 the default. Asking for it without a chooser must refuse
/// rather than quietly running at D0: a decision record naming a level that
/// did not run is false, and the record is what the audit rests on.
#[test]
fn d2_without_a_chooser_refuses_rather_than_falling_back() {
    if !appliance_built() {
        eprintln!("skipping: build the appliance first");
        return;
    }
    let out = drive("quit\n", &[]);

    assert!(out.contains("needs a chooser"), "{out}");
    assert!(
        !out.contains("autonomy     D0"),
        "it must not silently open at D0:\n{out}"
    );
    assert!(out.contains("--level d0"), "and should say how to:\n{out}");
}

#[test]
fn d0_runs_with_no_model_at_all() {
    if !appliance_built() {
        eprintln!("skipping: build the appliance first");
        return;
    }
    let out = drive("cycle\nquit\n", &["--level", "d0"]);

    assert!(out.contains("autonomy     D0"), "{out}");
    assert!(out.contains("accepted"), "the appliance accepted it:\n{out}");
}

/// D1: the model picks the tool, the engine computes the value.
#[test]
fn d1_opens_and_the_engine_still_sizes_the_correction() {
    let config = setup!();
    let out = drive(
        "cycle\nquit\n",
        &["--config", config.to_str().unwrap(), "--level", "d1"],
    );

    assert!(out.contains("autonomy     D1"), "{out}");
    assert!(out.contains("accepted"), "{out}");
}

/// D2: the model picks the tool and proposes the value.
#[test]
fn d2_opens_and_the_model_proposes_the_value() {
    let config = setup!();
    let out = drive(
        "cycle\nquit\n",
        &["--config", config.to_str().unwrap(), "--level", "d2"],
    );

    assert!(out.contains("autonomy     D2"), "{out}");
    assert!(out.contains("chooser"), "the chooser should be named:\n{out}");
    assert!(out.contains("accepted"), "and the appliance accepted it:\n{out}");
}

/// The levels must actually differ. If D2 produced the same value as D0, the
/// chooser would not be contributing and the test above would pass while
/// proving nothing.
#[test]
fn d2_sizes_the_correction_differently_from_d0() {
    let config = setup!();

    let d0 = drive("cycle\nquit\n", &["--level", "d0"]);
    let d2 = drive(
        "cycle\nquit\n",
        &["--config", config.to_str().unwrap(), "--level", "d2"],
    );

    let (a, b) = (sent_value(&d0), sent_value(&d2));
    assert!(a.is_some() && b.is_some(), "both should send a value:\n{d0}\n{d2}");
    assert_ne!(
        a, b,
        "D2 proposed the same value as D0, so the chooser changed nothing"
    );
}

/// Whatever the model proposes is clamped to the declared range and snapped to
/// the declared resolution before anything is sent. The grind parameter is
/// declared in whole steps, so a proposal of 21.143 reaches the appliance as
/// an integer.
#[test]
fn a_proposed_value_is_snapped_to_the_declared_resolution() {
    let config = setup!();
    let out = drive(
        "cycle\nquit\n",
        &["--config", config.to_str().unwrap(), "--level", "d2"],
    );

    let value = sent_value(&out).expect("a value should be sent");
    assert_eq!(
        value,
        value.round(),
        "the grind is declared in whole steps, got {value}:\n{out}"
    );
    assert!(
        (0.0..=240.0).contains(&value),
        "and inside the declared range, got {value}"
    );
}

#[test]
fn an_unknown_level_is_refused() {
    let out = drive("quit\n", &["--level", "d9"]);
    assert!(out.contains("unknown level"), "{out}");
}
