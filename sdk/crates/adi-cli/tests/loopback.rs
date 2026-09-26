//! The two implementations, talking.
//!
//! Everything before this was each side agreeing with the specification
//! separately. This is the first time the engine's encoder meets the
//! appliance's decoder, and the only evidence that they agree with each other.
//!
//! The appliance is `sdk/c/example`, a real build of libadip with real
//! generated tables, driven over its stdin and stdout as a child process. It
//! is not a mock: every byte goes through the same framing, CRC, CBOR decode,
//! range validation, interlock check and stateful envelope that would run on
//! a machine.
//!
//! Build it first:
//!
//! ```sh
//! cd sdk/c && mingw32-make CC=gcc example
//! ```
//!
//! Without it these tests skip rather than fail, so `cargo test` does not
//! require a C toolchain.
//!
//! This lives in `adi-cli` rather than next to `adi-wire` because the example
//! appliance is an espresso machine, and `adi-wire` is one of the crates that
//! may not name a domain. The wire itself is tested against a fictional one in
//! `adi-wire/tests/messages.rs`; what is proven here is that a real declaration
//! and a real device agree with it.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use adi_protocol::ids::SemanticId;
use adi_protocol::invocation::RejectReason;
use adi_protocol::units::{Quantity, Unit};
use adi_wire::{decode, encode_framed, frame, Arg, Message, Num, Outcome};

// ------------------------------------------------------------- the harness

fn example_path() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../c/build");
    for name in ["example.exe", "example"] {
        let p = root.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

struct Appliance {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    buf: Vec<u8>,
}

impl Appliance {
    fn start() -> Option<Self> {
        let path = example_path()?;
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the example appliance should start");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        Some(Self { child, stdin, stdout, buf: Vec::new() })
    }

    fn send(&mut self, message: &Message) {
        let bytes = encode_framed(message).expect("encodes");
        self.stdin.write_all(&bytes).expect("the appliance should accept bytes");
        self.stdin.flush().unwrap();
    }

    /// Read one message, growing the buffer until a whole frame is present.
    fn recv(&mut self) -> Message {
        loop {
            if let frame::Found::Frame { start, end, consumed } = frame::find(&self.buf) {
                let message = decode(&self.buf[start..end]).expect("the appliance sent a valid message");
                self.buf.drain(..consumed);
                return message;
            }

            let mut chunk = [0u8; 512];
            let n = self.stdout.read(&mut chunk).expect("the appliance should reply");
            assert!(n > 0, "the appliance closed the link with no reply");
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Read until a response arrives, collecting whatever telemetry comes
    /// first. A brew reports its progress before it reports its outcome.
    fn recv_response(&mut self) -> (Message, Vec<Message>) {
        let mut telemetry = Vec::new();
        for _ in 0..4000 {
            match self.recv() {
                m @ Message::Response { .. } => return (m, telemetry),
                other => telemetry.push(other),
            }
        }
        panic!("no response after 4000 messages");
    }
}

impl Drop for Appliance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn call(id: u32, tool: &str, args: &[(&str, Arg)]) -> Message {
    let mut map = BTreeMap::new();
    for (name, arg) in args {
        map.insert((*name).to_string(), arg.clone());
    }
    Message::Invocation { id, tool: SemanticId::from(tool), args: map }
}

fn q(value: f64, unit: Unit) -> Arg {
    Arg::Quantity(Num::from_quantity(Quantity::new(value, unit)).expect("representable"))
}

macro_rules! appliance {
    () => {
        match Appliance::start() {
            Some(a) => a,
            None => {
                eprintln!("skipping: build it with `cd sdk/c && mingw32-make CC=gcc example`");
                return;
            }
        }
    };
}

// --------------------------------------------------------------- the tests

/// The first byte ever to cross between the two implementations.
#[test]
fn the_appliance_answers_a_heartbeat() {
    let mut a = appliance!();

    a.send(&Message::Heartbeat { busy: None, faulted: None });

    match a.recv() {
        Message::Heartbeat { busy, faulted } => {
            assert_eq!(busy, Some(false), "an idle machine is not busy");
            assert_eq!(faulted, Some(false), "and has not faulted");
        }
        other => panic!("expected a heartbeat reply, got {other:?}"),
    }
}

#[test]
fn a_valid_invocation_is_accepted_and_completed() {
    let mut a = appliance!();

    a.send(&call(1, "adi.espresso.grind.set_setting", &[("setting", q(21.0, Unit::Step))]));

    match a.recv() {
        Message::Response { id, outcome, reason } => {
            assert_eq!(id, 1);
            assert_eq!(outcome, Outcome::Accepted);
            assert_eq!(reason, None);
        }
        other => panic!("expected acceptance, got {other:?}"),
    }

    match a.recv() {
        Message::Response { outcome, .. } => assert_eq!(outcome, Outcome::Completed),
        other => panic!("expected completion, got {other:?}"),
    }
}

/// ADI-P 4.6 requirement 2: an out of range value is an engine defect, and the
/// appliance rejects it rather than clamping, because silent clamping hides
/// the fault forever.
#[test]
fn an_out_of_range_value_is_rejected_not_clamped() {
    let mut a = appliance!();

    // The declared range is 0 to 240 steps.
    a.send(&call(2, "adi.espresso.grind.set_setting", &[("setting", q(900.0, Unit::Step))]));

    match a.recv() {
        Message::Response { outcome, reason, .. } => {
            assert_eq!(outcome, Outcome::Rejected);
            assert_eq!(reason, Some(RejectReason::OutOfRange));
        }
        other => panic!("expected rejection, got {other:?}"),
    }
}

/// The protection added in this work, proven end to end for the first time.
///
/// 21 is inside the declared 0 to 240, so no range check can object. Only the
/// unit says the engine computed seconds and sent them to a step parameter.
/// Before quantities carried their unit, this arrived as a plausible number
/// and was obeyed.
#[test]
fn a_value_in_range_in_the_wrong_unit_is_rejected() {
    let mut a = appliance!();

    a.send(&call(3, "adi.espresso.grind.set_setting", &[("setting", q(21.0, Unit::S))]));

    match a.recv() {
        Message::Response { outcome, reason, .. } => {
            assert_eq!(outcome, Outcome::Rejected);
            assert_eq!(
                reason,
                Some(RejectReason::UnitMismatch),
                "seconds into a step parameter must be refused"
            );
        }
        other => panic!("expected rejection, got {other:?}"),
    }
}

#[test]
fn an_undeclared_tool_is_refused() {
    let mut a = appliance!();

    a.send(&call(4, "x.nobody.invented.this", &[]));

    match a.recv() {
        Message::Response { outcome, reason, .. } => {
            assert_eq!(outcome, Outcome::Rejected);
            assert_eq!(reason, Some(RejectReason::UnknownTool));
        }
        other => panic!("expected rejection, got {other:?}"),
    }
}

/// The whole loop: a command goes out, the machine works, measurements and an
/// event come back, and the operation completes.
///
/// This is what the engine needs and what it has never had. Every number here
/// crossed a real CBOR encoder, a real frame, a real CRC and a real decoder.
#[test]
fn a_brew_reports_what_it_did() {
    let mut a = appliance!();

    a.send(&call(5, "adi.espresso.brew.start", &[("stop_at_mass", q(36.0, Unit::G))]));

    match a.recv() {
        Message::Response { outcome, reason, .. } => {
            assert_eq!(
                outcome,
                Outcome::Accepted,
                "a long operation is accepted first; refused with {reason:?}"
            );
        }
        other => panic!("expected acceptance, got {other:?}"),
    }

    let (response, telemetry) = a.recv_response();

    match response {
        Message::Response { id, outcome, .. } => {
            assert_eq!(id, 5);
            assert_eq!(outcome, Outcome::Completed);
        }
        other => panic!("expected completion, got {other:?}"),
    }

    let mut samples = Vec::new();
    let mut events = Vec::new();
    for m in telemetry {
        match m {
            Message::Signals(s) => samples.extend(s),
            Message::Event { code, .. } => events.push(code),
            _ => {}
        }
    }

    assert!(
        events.iter().any(|c| c == "first_drip"),
        "first drip is what shot time is measured from, and it was not reported"
    );

    let mass: Vec<_> = samples.iter().filter(|s| s.signal == 0).collect();
    assert!(mass.len() > 10, "a shot should produce a curve, got {} points", mass.len());

    for s in &mass {
        assert_eq!(s.value.unit, Unit::G, "the mass signal is declared in grams");
    }

    let first = mass.first().unwrap().value.to_quantity().value;
    let last = mass.last().unwrap().value.to_quantity().value;
    assert!(first < 1.0, "a shot starts empty, not at {first} g");
    assert!(
        (last - 36.0).abs() < 1.0,
        "it should stop near the 36 g it was asked for, not {last} g"
    );

    let times: Vec<u32> = mass.iter().map(|s| s.t_ms).collect();
    assert!(
        times.windows(2).all(|w| w[1] > w[0]),
        "timestamps must advance"
    );
}

/// Two operations in sequence, to prove nothing is left behind between them.
#[test]
fn the_machine_is_usable_again_afterwards() {
    let mut a = appliance!();

    a.send(&call(6, "adi.espresso.brew.start", &[("stop_at_mass", q(30.0, Unit::G))]));
    let _ = a.recv();
    let (done, _) = a.recv_response();
    assert!(matches!(done, Message::Response { outcome: Outcome::Completed, .. }));

    a.send(&call(7, "adi.espresso.grind.set_setting", &[("setting", q(23.0, Unit::Step))]));
    match a.recv() {
        Message::Response { id, outcome, reason } => {
            assert_eq!(id, 7);
            assert_eq!(outcome, Outcome::Accepted, "reason: {reason:?}");
        }
        other => panic!("expected acceptance, got {other:?}"),
    }
}
