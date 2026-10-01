//! `adi run` against a real appliance.
//!
//! This is the only command that runs the engine rather than inspecting it, so
//! it is also the only one whose failure would leave the SDK demonstrably
//! broken while every other test passed.
//!
//! Driven by feeding commands on standard input, which is how a person uses
//! it. Skipped where the appliance has not been built.
//!
//! These run at D0 explicitly. The default is D2, which needs a chooser; what
//! is being tested here is the session and the appliance rather than the
//! autonomy levels, which have their own file.

use std::io::Write;
use std::process::{Command, Stdio};

fn appliance_built() -> bool {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../c/build");
    base.join("example.exe").exists() || base.join("example").exists()
}

/// Run `adi run` with the given lines typed at it, from the sdk directory.
fn drive(input: &str, extra: &[&str]) -> String {
    let sdk = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    let mut args = vec!["run"];
    args.extend_from_slice(extra);

    let mut child = Command::new(env!("CARGO_BIN_EXE_adi"))
        .args(&args)
        .current_dir(&sdk)
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

macro_rules! require_appliance {
    () => {
        if !appliance_built() {
            eprintln!("skipping: build it with `cd sdk/c && make example`");
            return;
        }
    };
}

#[test]
fn a_session_opens_against_a_real_appliance() {
    require_appliance!();
    let out = drive("quit\n", &["--level", "d0"]);

    assert!(out.contains("EX-900"), "the machine should be named:\n{out}");
    assert!(out.contains("autonomy"), "the level should be stated:\n{out}");
    assert!(out.contains("closed"), "it should close cleanly:\n{out}");
}

/// The whole loop with no model involved: a finished cycle is observed, the
/// profile diagnoses it, the engine plans a correction, the safety envelope
/// passes it, and a real libadip build accepts the invocation.
#[test]
fn a_cycle_is_observed_diagnosed_and_corrected() {
    require_appliance!();
    let out = drive("cycle\nquit\n", &["--level", "d0"]);

    assert!(out.contains("observing"), "{out}");
    assert!(out.contains("diagnosed"), "it should reach a diagnosis:\n{out}");
    assert!(
        out.contains("adi.espresso.grind.set_setting"),
        "it should choose the grind tool:\n{out}"
    );
    assert!(
        out.contains("accepted"),
        "the appliance should accept the invocation:\n{out}"
    );
}

/// The regression this test exists for. A cycle records `grind_setting` while
/// the tool that moves it declares a parameter called `setting`. Matching
/// those by exact name only, which the session did at first, produced
/// "no present value for parameter 'setting'" and no correction at all.
///
/// It was invisible to the session's own tests because the neutral fixture
/// names its variables identically to its parameters.
#[test]
fn a_setting_named_for_its_variable_still_reaches_the_planner() {
    require_appliance!();
    let out = drive("cycle\nquit\n", &["--level", "d0"]);

    assert!(
        !out.contains("no present value"),
        "the planner should find the current setting:\n{out}"
    );
    assert!(
        out.contains("setting ="),
        "and an absolute value should be sent:\n{out}"
    );
}

#[test]
fn state_carries_across_cycles() {
    require_appliance!();
    let out = drive("cycle\ncycle\nstatus\nquit\n", &["--level", "d0"]);

    assert!(
        out.contains("cycles held  2"),
        "both cycles should be remembered:\n{out}"
    );
    assert!(
        out.contains("decisions    2"),
        "and both decisions recorded:\n{out}"
    );
}

#[test]
fn the_heartbeat_reaches_the_appliance() {
    require_appliance!();
    let out = drive("status\nquit\n", &["--level", "d0"]);
    assert!(out.contains("link         alive"), "{out}");
}

/// ADI-P 9.4.2 permits an engine with no model, provided it names what it
/// cannot do rather than failing.
#[test]
fn without_an_interpreter_a_spoken_request_says_so() {
    require_appliance!();
    let out = drive("make me a lungo\nquit\n", &["--level", "d0"]);

    assert!(
        out.contains("no interpreter"),
        "it should name what is missing:\n{out}"
    );
    assert!(
        out.contains("adi models check"),
        "and say how to find out more:\n{out}"
    );
    assert!(out.contains("closed"), "and still close cleanly:\n{out}");
}

/// A path written without a platform's executable suffix still resolves, so
/// one documented command works on every platform.
#[test]
fn an_appliance_path_without_the_exe_suffix_resolves() {
    require_appliance!();
    let out = drive(
        "quit
",
        &["--level", "d0", "--appliance", "c/build/example"],
    );

    assert!(
        out.contains("autonomy"),
        "the session should open from a suffixless path:
{out}"
    );
    assert!(!out.contains("no appliance at"), "{out}");
}

#[test]
fn a_missing_appliance_explains_how_to_build_it() {
    let out = drive("quit\n", &["--appliance", "no/such/appliance"]);
    assert!(out.contains("make example"), "{out}");
}

/// A refusal is reported and the session continues.
///
/// Every command used to propagate its error, so the first refusal ended the
/// session. That is wrong twice over: a refusal is frequently the correct
/// outcome, and a person who has just been refused is the one most likely to
/// want to ask something else.
#[test]
fn a_refusal_does_not_end_the_session() {
    require_appliance!();
    let Some(py) = python() else {
        eprintln!("skipping: no python on PATH");
        return;
    };

    let config = config_for(py, "examples/interpreters/unservable_request.py");
    let out = drive(
        "raise the extraction
status
quit
",
        &["--level", "d0", "--config", config.to_str().unwrap()],
    );

    assert!(
        out.contains("cannot") || out.contains("nothing this machine"),
        "the refusal should be explained:
{out}"
    );
    assert!(
        out.contains("link         alive"),
        "and the next command should still run:
{out}"
    );
    assert!(out.contains("closed"), "and the session close cleanly:
{out}");
    assert!(
        !out.contains("NoCandidate"),
        "a person should not be shown the raw error:
{out}"
    );
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

fn config_for(py: &str, script: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir()
        .join(format!("adi-run-{}-{unique}.yaml", std::process::id()));
    std::fs::write(
        &path,
        format!(
            "root: .
seams:
  interpreter:
    backend: process
    command: {py}
    args: [\"{script}\"]
"
        ),
    )
    .unwrap();
    path
}
