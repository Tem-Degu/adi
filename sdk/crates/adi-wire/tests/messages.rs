//! Byte level agreement with `libadip`.
//!
//! These assert the exact bytes rather than a round trip through this crate
//! alone, because a round trip proves only that this implementation is
//! consistent with itself. The two implementations were consistent with
//! themselves for months while disagreeing with each other about the type of
//! an invocation id and the set of reason codes, and nothing caught it,
//! because nothing compared them.
//!
//! Every expected sequence here is derived by hand from ADI-P 10.2 and matches
//! what `sdk/c/test/*.c` builds and what `adip_decode.c` emits.

use std::collections::BTreeMap;

use adi_protocol::ids::SemanticId;
use adi_protocol::invocation::{Invocation, RejectReason};
use adi_protocol::units::{Quantity, Unit};
use adi_wire::{decode, encode, peek_kind, Arg, Kind, Message, Num, Outcome, Sample, WireError};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X} ")).collect::<String>().trim_end().to_string()
}

/// The frame `make_heartbeat` builds in `sdk/c/test/test_m5.c`:
/// a map of one, key 0, value 5.
#[test]
fn a_heartbeat_is_three_bytes() {
    let bytes = encode(&Message::Heartbeat { busy: None, faulted: None }).unwrap();
    assert_eq!(hex(&bytes), "A1 00 05");
}

#[test]
fn a_heartbeat_reply_carries_both_flags() {
    let bytes = encode(&Message::Heartbeat {
        busy: Some(true),
        faulted: Some(false),
    })
    .unwrap();
    // map(3), 0:5, 1:true, 2:false. CBOR simple 21 is true, 20 is false.
    assert_eq!(hex(&bytes), "A3 00 05 01 F5 02 F4");
}

/// 36.5 g is `[1, 4([-1, 365])]`: the unit, then tag 4 over exponent and
/// mantissa. This is the shape `adip_num_t` has in memory, which is the whole
/// reason CBOR was chosen over JSON.
#[test]
fn a_quantity_matches_adip_num_t() {
    let mut args = BTreeMap::new();
    args.insert(
        "mass".to_string(),
        Arg::Quantity(Num::from_quantity(Quantity::new(36.5, Unit::G)).unwrap()),
    );

    let bytes = encode(&Message::Invocation {
        id: 1,
        tool: SemanticId::from("a"),
        args,
    })
    .unwrap();

    let expected = concat!(
        "A4 ",             // map of 4
        "00 01 ",          // kind: invocation
        "01 61 61 ",       // tool: "a"
        "02 01 ",          // id: 1
        "03 A1 ",          // args: map of 1
        "64 6D 61 73 73 ", // "mass"
        "82 01 ",          // [unit g,
        "C4 82 20 19 01 6D" //  tag4([-1, 365])]
    );
    assert_eq!(hex(&bytes), expected);
}

#[test]
fn an_invocation_with_no_arguments() {
    let bytes = encode(&Message::Invocation {
        id: 7,
        tool: SemanticId::from("a"),
        args: BTreeMap::new(),
    })
    .unwrap();
    assert_eq!(hex(&bytes), "A4 00 01 01 61 61 02 07 03 A0");
}

/// What `adip_encode_response` emits for an accepted call: the kind, the id
/// and the outcome, with the reason omitted because there is none.
#[test]
fn an_accepted_response_omits_the_reason() {
    let bytes = encode(&Message::Response {
        id: 7,
        outcome: Outcome::Accepted,
        reason: None,
    })
    .unwrap();
    assert_eq!(hex(&bytes), "A3 00 02 01 07 02 01");
}

#[test]
fn a_rejection_carries_its_numbered_reason() {
    let bytes = encode(&Message::Response {
        id: 7,
        outcome: Outcome::Rejected,
        reason: Some(RejectReason::UnitMismatch),
    })
    .unwrap();
    // map(4), kind 2, id 7, outcome 5, reason 13.
    assert_eq!(hex(&bytes), "A4 00 02 01 07 02 05 03 0D");
}

// ------------------------------------------------------------- round trips

#[test]
fn every_kind_survives_a_round_trip() {
    let mut args = BTreeMap::new();
    args.insert(
        "mass".to_string(),
        Arg::Quantity(Num::from_quantity(Quantity::new(9.0, Unit::G)).unwrap()),
    );
    args.insert("substance".to_string(), Arg::Text("example".to_string()));

    let messages = vec![
        Message::Invocation {
            id: 41,
            tool: SemanticId::from("adi.example.input.by_mass"),
            args,
        },
        Message::Response {
            id: 41,
            outcome: Outcome::Completed,
            reason: None,
        },
        Message::Response {
            id: 41,
            outcome: Outcome::Rejected,
            reason: Some(RejectReason::Interlock),
        },
        Message::Signals(vec![
            Sample {
                signal: 0,
                value: Num::from_quantity(Quantity::new(36.5, Unit::G)).unwrap(),
                t_ms: 1234,
            },
            Sample {
                signal: 1,
                value: Num::from_quantity(Quantity::new(28.4, Unit::S)).unwrap(),
                t_ms: 1284,
            },
        ]),
        Message::Event {
            index: 0,
            t_ms: 4242,
            code: "overheated".to_string(),
            severity: 2,
        },
        Message::Heartbeat { busy: None, faulted: None },
        Message::Heartbeat {
            busy: Some(true),
            faulted: Some(true),
        },
    ];

    for original in messages {
        let bytes = encode(&original).expect("encodes");
        let back = decode(&bytes).expect("decodes");
        assert_eq!(back, original, "round trip changed the message");
    }
}

#[test]
fn an_engine_invocation_becomes_a_message() {
    let call = Invocation::new(SemanticId::from("adi.example.input.by_mass"), 41)
        .with_number("mass", Quantity::new(9.0, Unit::G))
        .with_text("substance", "example");

    let message = adi_wire::invocation(&call).expect("converts");
    let back = decode(&encode(&message).unwrap()).unwrap();

    match back {
        Message::Invocation { id, tool, args } => {
            assert_eq!(id, 41);
            assert_eq!(tool.as_str(), "adi.example.input.by_mass");
            assert_eq!(
                args["mass"],
                Arg::Quantity(Num::new(9, 0, Unit::G)),
                "the unit travels with the value"
            );
            assert_eq!(args["substance"], Arg::Text("example".to_string()));
        }
        other => panic!("expected an invocation, got {other:?}"),
    }
}

// ---------------------------------------------------------------- refusals

/// The refusal that keeps range checks exact. A binary float cannot hold 0.1,
/// so accepting one would put back the ambiguity the decimal form removes.
#[test]
fn a_binary_float_is_refused() {
    // map(4), kind 1, tool "a", id 1, args { "m": <float64 0.0> }
    let mut bytes = vec![
        0xA4, 0x00, 0x01, 0x01, 0x61, 0x61, 0x02, 0x01, 0x03, 0xA1, 0x61, 0x6D,
    ];
    bytes.push(0x82); // a well formed quantity ...
    bytes.push(0x01); // ... in grams ...
    bytes.push(0xFB); // ... carrying a float64
    bytes.extend_from_slice(&[0; 8]);

    assert_eq!(decode(&bytes), Err(WireError::BinaryFloat));
}

#[test]
fn an_undefined_unit_is_refused() {
    let mut bytes = vec![
        0xA4, 0x00, 0x01, 0x01, 0x61, 0x61, 0x02, 0x01, 0x03, 0xA1, 0x61, 0x6D,
    ];
    bytes.extend_from_slice(&[0x82, 0x18, 200]); // unit 200, which is not defined
    bytes.extend_from_slice(&[0xC4, 0x82, 0x20, 0x09]);

    assert_eq!(decode(&bytes), Err(WireError::UnknownUnit(200)));
}

#[test]
fn a_message_with_no_kind_is_not_a_control_message() {
    // map(1), key 1, text "a": well formed CBOR, not a control message.
    assert_eq!(decode(&[0xA1, 0x01, 0x61, 0x61]), Err(WireError::NoKind));
}

#[test]
fn an_unknown_kind_is_refused_rather_than_guessed() {
    assert_eq!(decode(&[0xA1, 0x00, 0x09]), Err(WireError::UnknownKind(9)));
}

#[test]
fn a_truncated_message_is_refused() {
    let full = encode(&Message::Response {
        id: 7,
        outcome: Outcome::Accepted,
        reason: None,
    })
    .unwrap();

    for cut in 1..full.len() {
        assert!(
            decode(&full[..cut]).is_err(),
            "a message cut to {cut} bytes should not decode"
        );
    }
}

/// ADI-P 11 requires tolerating unknown optional fields within a major
/// version, so a newer engine may add a key without breaking an older device.
#[test]
fn an_unknown_key_is_ignored() {
    // map(4): kind, id, outcome, and key 99 carrying text.
    let bytes = vec![
        0xA4, 0x00, 0x02, 0x01, 0x07, 0x02, 0x01, 0x18, 99, 0x61, 0x7A,
    ];
    assert_eq!(
        decode(&bytes).unwrap(),
        Message::Response {
            id: 7,
            outcome: Outcome::Accepted,
            reason: None
        }
    );
}

#[test]
fn peeking_the_kind_does_not_need_the_body() {
    let bytes = encode(&Message::Signals(vec![Sample {
        signal: 0,
        value: Num::new(1, 0, Unit::G),
        t_ms: 1,
    }]))
    .unwrap();
    assert_eq!(peek_kind(&bytes).unwrap(), Kind::Signals);
}
