//! How a session reaches an appliance.
//!
//! One trait, because ADI-P 10 lists four bindings and the engine should not
//! know which one it has. Shared memory, a UART, BLE and a child process over
//! a pipe differ in how bytes move and in nothing above that.

use adi_wire::Message;

use crate::SessionError;

/// A link to one appliance.
///
/// Implementations must not block indefinitely. A session that cannot be
/// interrupted cannot send the heartbeat that keeps the appliance's failsafe
/// from firing, which would make the transport the cause of the fault it was
/// supposed to avoid.
pub trait Transport {
    /// Send one message. Framing belongs to the implementation, because a
    /// binding that already frames, such as BLE GATT, must not frame twice.
    fn send(&mut self, message: &Message) -> Result<(), SessionError>;

    /// Take the next message if one has arrived within `timeout_ms`.
    ///
    /// `Ok(None)` means nothing arrived, which is ordinary and not an error:
    /// an appliance is silent for most of a long operation.
    fn recv(&mut self, timeout_ms: u32) -> Result<Option<Message>, SessionError>;
}

/// A shared view of what a session sent.
///
/// The session takes ownership of its transport, so a test that wants to
/// assert on traffic needs a handle that outlives the handover. Cloning this
/// shares the same log rather than copying it.
#[derive(Clone, Default)]
pub struct Log(std::rc::Rc<std::cell::RefCell<Vec<Message>>>);

impl Log {
    pub fn messages(&self) -> Vec<Message> {
        self.0.borrow().clone()
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many of one kind were sent. Heartbeats are the usual question:
    /// too few and the appliance's failsafe fires mid-operation, too many and
    /// a UART carrying real work is flooded with liveness.
    pub fn count(&self, kind: adi_wire::Kind) -> usize {
        self.0
            .borrow()
            .iter()
            .filter(|m| adi_wire::kind_of(m) == kind)
            .count()
    }

    pub fn invocations(&self) -> Vec<Message> {
        self.0
            .borrow()
            .iter()
            .filter(|m| matches!(m, Message::Invocation { .. }))
            .cloned()
            .collect()
    }
}

/// A transport that records what it was given and replies from a script.
///
/// For testing a session without a device. It is in the library rather than in
/// the tests because an integrator writing their own `Transport` needs
/// something to check their session logic against before they have hardware,
/// and that is the same need.
#[derive(Default)]
pub struct Recorded {
    log: Log,
    replies: std::collections::VecDeque<Option<Message>>,
}

impl Recorded {
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle to what this transport records, to keep after the session
    /// takes ownership.
    pub fn log(&self) -> Log {
        self.log.clone()
    }

    /// Queue a reply for the session to receive.
    pub fn will_reply(&mut self, message: Message) -> &mut Self {
        self.replies.push_back(Some(message));
        self
    }

    /// Queue a silence: one read that finds nothing waiting.
    ///
    /// A real link runs dry between the things an appliance has to say, and a
    /// reader that drains until empty needs that to be expressible. Without
    /// it a queue of replies looks like one unbroken burst, and a test cannot
    /// say that a report arrived during a heartbeat while the reply to a later
    /// invocation had not been sent yet.
    pub fn will_fall_silent(&mut self) -> &mut Self {
        self.replies.push_back(None);
        self
    }
}

impl Transport for Recorded {
    fn send(&mut self, message: &Message) -> Result<(), SessionError> {
        self.log.0.borrow_mut().push(message.clone());
        Ok(())
    }

    fn recv(&mut self, _timeout_ms: u32) -> Result<Option<Message>, SessionError> {
        // An exhausted queue and a queued silence read the same way, which is
        // what a link with nothing on it looks like.
        Ok(self.replies.pop_front().flatten())
    }
}
