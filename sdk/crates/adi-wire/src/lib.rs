//! ADI-P 10.2 message encoding: the engine side of the wire `libadip` speaks.
//!
//! The appliance side of this protocol has existed and been tested for some
//! time. This side did not, so the two implementations had never exchanged a
//! byte, and the disagreements between them were found by reading rather than
//! by running. That is what this crate exists to end.
//!
//! Nothing here is domain specific and nothing may become so. A frame carrying
//! a coffee measurement and one carrying a greenhouse reading differ only in
//! which index the declaration gave the signal.

pub mod cbor;
pub mod frame;
pub mod num;

use std::collections::BTreeMap;

use adi_protocol::ids::SemanticId;
use adi_protocol::invocation::{Invocation, RejectReason};
use adi_protocol::units::Quantity;

pub use num::Num;

/// Message kinds, ADI-P 10.2.4. The discriminants are the wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Invocation = 1,
    Response = 2,
    Signals = 3,
    Event = 4,
    Heartbeat = 5,
    Setpoints = 6,
}

impl Kind {
    pub fn from_wire(v: u64) -> Option<Self> {
        Some(match v {
            1 => Kind::Invocation,
            2 => Kind::Response,
            3 => Kind::Signals,
            4 => Kind::Event,
            5 => Kind::Heartbeat,
            6 => Kind::Setpoints,
            _ => return None,
        })
    }
}

/// Lifecycle states, ADI-P 9.2 and 10.2.6. Zero is deliberately not one of
/// them, so a zeroed buffer cannot read as a meaningful outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Outcome {
    Accepted = 1,
    Running = 2,
    Completed = 3,
    Aborted = 4,
    Rejected = 5,
}

impl Outcome {
    pub fn from_wire(v: u64) -> Option<Self> {
        Some(match v {
            1 => Outcome::Accepted,
            2 => Outcome::Running,
            3 => Outcome::Completed,
            4 => Outcome::Aborted,
            5 => Outcome::Rejected,
            _ => return None,
        })
    }
}

/// One measurement, as the appliance reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Index into the declaration's `signals`.
    pub signal: u16,
    pub value: Num,
    /// The appliance's own monotonic clock. Relative: two devices share no
    /// epoch, and treating this as absolute would put a fabricated time into a
    /// decision record.
    pub t_ms: u32,
}

/// One parameter's setpoint, as the appliance reports it.
///
/// Not a [`Sample`]. A sample is something the machine measured; this is what
/// a mechanism is commanded to. A heater told to reach 62 C may be measuring
/// 45 C on the way there, and the planner adjusts the command rather than the
/// thermometer. Conflating the two would have the engine plan from a reading
/// that is still moving.
///
/// Addressed by position, as events and signals already are: the declaration
/// gives both sides the same ordered list of tools, and each tool the same
/// ordered list of parameters. No names cross the wire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setpoint {
    /// Index into the declaration's `tools`.
    pub tool: u16,
    /// Index into that tool's parameters, in declaration order.
    pub parameter: u16,
    pub value: Num,
}

/// An argument value in an invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Quantity(Num),
    Text(String),
}

/// One control message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Invocation {
        id: u32,
        tool: SemanticId,
        args: BTreeMap<String, Arg>,
    },
    Response {
        id: u32,
        outcome: Outcome,
        reason: Option<RejectReason>,
    },
    Signals(Vec<Sample>),
    Event {
        index: u16,
        t_ms: u32,
        code: String,
        severity: u8,
    },
    /// Outbound from the engine the flags are absent; the appliance's reply
    /// carries them.
    Heartbeat {
        busy: Option<bool>,
        faulted: Option<bool>,
    },
    /// What the appliance's mechanisms are currently commanded to.
    ///
    /// ADI-P 9.1 requires that an operator can take control by physical means
    /// at any moment, so a setting can change with the engine neither asking
    /// nor being told. Anything the engine remembers is therefore stale by
    /// mandate, and this is how it stops guessing: the appliance owns its own
    /// state, as it already owns its interlocks, its preconditions and whether
    /// it is faulted.
    ///
    /// Sent unsolicited, when a session opens and whenever a value changes.
    /// There is no engine-initiated query anywhere in this protocol and this
    /// does not add one.
    Setpoints(Vec<Setpoint>),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WireError {
    #[error("the message ended early")]
    Truncated,
    #[error("malformed: {0}")]
    Malformed(&'static str),
    #[error("a binary float cannot represent a decimal exactly and is refused")]
    BinaryFloat,
    #[error("no unit is defined for wire value {0}")]
    UnknownUnit(u64),
    #[error("no message kind is defined for wire value {0}")]
    UnknownKind(u64),
    #[error("no outcome is defined for wire value {0}")]
    UnknownOutcome(u64),
    #[error("the message carries no kind, so it is not a control message")]
    NoKind,
    #[error("the value is not finite")]
    NotFinite,
    #[error("{0} does not fit a 32 bit mantissa")]
    OutOfRange(f64),
    #[error("{0} needs more decimal places than the wire carries")]
    NotRepresentable(f64),
    #[error("a payload of {0} bytes exceeds the maximum frame")]
    TooLong(usize),
}

/// Which kind a message is, without matching every arm.
pub fn kind_of(message: &Message) -> Kind {
    match message {
        Message::Invocation { .. } => Kind::Invocation,
        Message::Response { .. } => Kind::Response,
        Message::Signals(_) => Kind::Signals,
        Message::Event { .. } => Kind::Event,
        Message::Heartbeat { .. } => Kind::Heartbeat,
        Message::Setpoints(_) => Kind::Setpoints,
    }
}

/// Reserved map key holding the kind. ADI-P 10.2.4.
const KEY_KIND: u64 = 0;

// ----------------------------------------------------------------- encoding

/// Encode a message, without framing.
pub fn encode(message: &Message) -> Result<Vec<u8>, WireError> {
    let mut w = cbor::Writer::new();

    match message {
        Message::Invocation { id, tool, args } => {
            w.map(4);
            w.uint(KEY_KIND);
            w.uint(Kind::Invocation as u64);
            w.uint(1);
            w.text(tool.as_str());
            w.uint(2);
            w.uint(u64::from(*id));
            w.uint(3);
            w.map(args.len() as u64);
            for (name, arg) in args {
                w.text(name);
                match arg {
                    Arg::Quantity(n) => w.quantity(*n),
                    Arg::Text(t) => w.text(t),
                }
            }
        }

        Message::Response { id, outcome, reason } => {
            // The reason is omitted when there is none, so a response carries
            // three entries or four.
            w.map(if reason.is_some() { 4 } else { 3 });
            w.uint(KEY_KIND);
            w.uint(Kind::Response as u64);
            w.uint(1);
            w.uint(u64::from(*id));
            w.uint(2);
            w.uint(*outcome as u64);
            if let Some(r) = reason {
                w.uint(3);
                w.uint(u64::from(r.as_wire()));
            }
        }

        Message::Signals(samples) => {
            w.map(2);
            w.uint(KEY_KIND);
            w.uint(Kind::Signals as u64);
            w.uint(1);
            w.array(samples.len() as u64);
            for s in samples {
                w.array(3);
                w.uint(u64::from(s.signal));
                w.quantity(s.value);
                w.uint(u64::from(s.t_ms));
            }
        }

        Message::Setpoints(setpoints) => {
            w.map(2);
            w.uint(KEY_KIND);
            w.uint(Kind::Setpoints as u64);
            w.uint(1);
            w.array(setpoints.len() as u64);
            for sp in setpoints {
                w.array(3);
                w.uint(u64::from(sp.tool));
                w.uint(u64::from(sp.parameter));
                w.quantity(sp.value);
            }
        }

        Message::Event { index, t_ms, code, severity } => {
            w.map(5);
            w.uint(KEY_KIND);
            w.uint(Kind::Event as u64);
            w.uint(1);
            w.uint(u64::from(*index));
            w.uint(2);
            w.uint(u64::from(*t_ms));
            w.uint(3);
            w.text(code);
            w.uint(4);
            w.uint(u64::from(*severity));
        }

        Message::Heartbeat { busy, faulted } => match (busy, faulted) {
            (Some(b), Some(f)) => {
                w.map(3);
                w.uint(KEY_KIND);
                w.uint(Kind::Heartbeat as u64);
                w.uint(1);
                w.bool(*b);
                w.uint(2);
                w.bool(*f);
            }
            _ => {
                w.map(1);
                w.uint(KEY_KIND);
                w.uint(Kind::Heartbeat as u64);
            }
        },
    }

    if w.out.len() > frame::MAX_PAYLOAD {
        return Err(WireError::TooLong(w.out.len()));
    }

    Ok(w.out)
}

/// Encode a message and wrap it for a stream transport.
pub fn encode_framed(message: &Message) -> Result<Vec<u8>, WireError> {
    frame::wrap(&encode(message)?)
}

/// Build an invocation message from the engine's own type.
///
/// Every numeric argument becomes a quantity carrying its unit, which is what
/// lets the appliance refuse a value computed in the wrong one.
pub fn invocation(call: &Invocation) -> Result<Message, WireError> {
    let mut args: BTreeMap<String, Arg> = BTreeMap::new();

    for (name, q) in &call.numeric_arguments {
        args.insert(name.clone(), Arg::Quantity(Num::from_quantity(*q)?));
    }
    for (name, text) in &call.text_arguments {
        args.insert(name.clone(), Arg::Text(text.clone()));
    }

    Ok(Message::Invocation {
        id: call.invocation_id,
        tool: call.tool.clone(),
        args,
    })
}

// ----------------------------------------------------------------- decoding

/// Decode one message payload.
pub fn decode(payload: &[u8]) -> Result<Message, WireError> {
    // The kind is read first, on its own pass, because a sender is not
    // required to order the keys and the body cannot be interpreted until the
    // kind is known. The C decoder makes two passes for the same reason.
    let kind = peek_kind(payload)?;

    let mut r = cbor::Reader::new(payload);
    let count = r.map()?;

    match kind {
        Kind::Invocation => decode_invocation(&mut r, count),
        Kind::Response => decode_response(&mut r, count),
        Kind::Signals => decode_signals(&mut r, count),
        Kind::Event => decode_event(&mut r, count),
        Kind::Heartbeat => decode_heartbeat(&mut r, count),
        Kind::Setpoints => decode_setpoints(&mut r, count),
    }
}

/// Read only the kind, so a caller can route before decoding.
pub fn peek_kind(payload: &[u8]) -> Result<Kind, WireError> {
    let mut r = cbor::Reader::new(payload);
    let count = r.map()?;

    for _ in 0..count {
        let key = r.uint()?;
        if key == KEY_KIND {
            let v = r.uint()?;
            return Kind::from_wire(v).ok_or(WireError::UnknownKind(v));
        }
        r.skip()?;
    }

    Err(WireError::NoKind)
}

fn decode_invocation(r: &mut cbor::Reader, count: u64) -> Result<Message, WireError> {
    let mut id = 0u32;
    let mut tool: Option<String> = None;
    let mut args: BTreeMap<String, Arg> = BTreeMap::new();

    for _ in 0..count {
        match r.uint()? {
            KEY_KIND => {
                r.skip()?;
            }
            1 => tool = Some(r.text()?.to_string()),
            2 => {
                id = u32::try_from(r.uint()?)
                    .map_err(|_| WireError::Malformed("invocation id is not 32 bit"))?
            }
            3 => {
                let n = r.array_or_map_len()?;
                for _ in 0..n {
                    let name = r.text()?.to_string();
                    args.insert(name, r.arg()?);
                }
            }
            // Unknown keys are ignored so a minor version may add optional
            // fields. ADI-P 11.
            _ => r.skip()?,
        }
    }

    Ok(Message::Invocation {
        id,
        tool: SemanticId(tool.ok_or(WireError::Malformed("no tool named"))?),
        args,
    })
}

fn decode_response(r: &mut cbor::Reader, count: u64) -> Result<Message, WireError> {
    let mut id = 0u32;
    let mut outcome = None;
    let mut reason = None;

    for _ in 0..count {
        match r.uint()? {
            KEY_KIND => {
                r.skip()?;
            }
            1 => {
                id = u32::try_from(r.uint()?)
                    .map_err(|_| WireError::Malformed("invocation id is not 32 bit"))?
            }
            2 => {
                let v = r.uint()?;
                outcome = Some(Outcome::from_wire(v).ok_or(WireError::UnknownOutcome(v))?);
            }
            3 => {
                let v = r.uint()?;
                // An unrecognised reason is not fatal: the response still says
                // the call was refused, and inventing a meaning would be worse
                // than reporting that the code is unknown to this version.
                reason = u8::try_from(v).ok().and_then(RejectReason::from_wire);
            }
            _ => r.skip()?,
        }
    }

    Ok(Message::Response {
        id,
        outcome: outcome.ok_or(WireError::Malformed("no outcome given"))?,
        reason,
    })
}

fn decode_signals(r: &mut cbor::Reader, count: u64) -> Result<Message, WireError> {
    let mut samples = Vec::new();

    for _ in 0..count {
        match r.uint()? {
            KEY_KIND => {
                r.skip()?;
            }
            1 => {
                let n = r.array()?;
                for _ in 0..n {
                    if r.array()? != 3 {
                        return Err(WireError::Malformed("a sample is [signal, value, t_ms]"));
                    }
                    let signal = u16::try_from(r.uint()?)
                        .map_err(|_| WireError::Malformed("signal index is not 16 bit"))?;
                    let value = r.quantity()?;
                    let t_ms = u32::try_from(r.uint()?)
                        .map_err(|_| WireError::Malformed("timestamp is not 32 bit"))?;
                    samples.push(Sample { signal, value, t_ms });
                }
            }
            _ => r.skip()?,
        }
    }

    Ok(Message::Signals(samples))
}

fn decode_setpoints(r: &mut cbor::Reader, count: u64) -> Result<Message, WireError> {
    let mut setpoints = Vec::new();

    for _ in 0..count {
        match r.uint()? {
            KEY_KIND => {
                r.skip()?;
            }
            1 => {
                let n = r.array()?;
                for _ in 0..n {
                    if r.array()? != 3 {
                        return Err(WireError::Malformed(
                            "a setpoint is [tool, parameter, value]",
                        ));
                    }
                    let tool = u16::try_from(r.uint()?)
                        .map_err(|_| WireError::Malformed("tool index is not 16 bit"))?;
                    let parameter = u16::try_from(r.uint()?)
                        .map_err(|_| WireError::Malformed("parameter index is not 16 bit"))?;
                    let value = r.quantity()?;
                    setpoints.push(Setpoint { tool, parameter, value });
                }
            }
            _ => r.skip()?,
        }
    }

    Ok(Message::Setpoints(setpoints))
}

fn decode_event(r: &mut cbor::Reader, count: u64) -> Result<Message, WireError> {
    let mut index = 0u16;
    let mut t_ms = 0u32;
    let mut code = String::new();
    let mut severity = 0u8;

    for _ in 0..count {
        match r.uint()? {
            KEY_KIND => {
                r.skip()?;
            }
            1 => {
                index = u16::try_from(r.uint()?)
                    .map_err(|_| WireError::Malformed("event index is not 16 bit"))?
            }
            2 => {
                t_ms = u32::try_from(r.uint()?)
                    .map_err(|_| WireError::Malformed("timestamp is not 32 bit"))?
            }
            3 => code = r.text()?.to_string(),
            4 => {
                severity = u8::try_from(r.uint()?)
                    .map_err(|_| WireError::Malformed("severity is not 8 bit"))?
            }
            _ => r.skip()?,
        }
    }

    Ok(Message::Event { index, t_ms, code, severity })
}

fn decode_heartbeat(r: &mut cbor::Reader, count: u64) -> Result<Message, WireError> {
    let mut busy = None;
    let mut faulted = None;

    for _ in 0..count {
        match r.uint()? {
            KEY_KIND => {
                r.skip()?;
            }
            1 => busy = Some(r.bool()?),
            2 => faulted = Some(r.bool()?),
            _ => r.skip()?,
        }
    }

    Ok(Message::Heartbeat { busy, faulted })
}

impl cbor::Reader<'_> {
    /// An argument is a quantity or a text value, and which one is decided by
    /// the shape rather than by the declaration, so a decoder needs no tables.
    fn arg(&mut self) -> Result<Arg, WireError> {
        let save = self.pos;
        let (mt, _) = self.head()?;
        self.pos = save;

        if mt == cbor::MT_TEXT {
            Ok(Arg::Text(self.text()?.to_string()))
        } else {
            Ok(Arg::Quantity(self.quantity()?))
        }
    }

    fn array_or_map_len(&mut self) -> Result<u64, WireError> {
        let save = self.pos;
        let (mt, value) = self.head()?;
        if mt == cbor::MT_MAP || mt == cbor::MT_ARRAY {
            Ok(value)
        } else {
            self.pos = save;
            Err(WireError::Malformed("arguments must be a map"))
        }
    }
}

/// Convert a decoded quantity back into the engine's own type.
pub fn to_quantity(n: Num) -> Quantity {
    n.to_quantity()
}
