//! What the machine remembers.
//!
//! "Half the sugar of last time" is unanswerable without this, and it is the
//! ordinary way people talk to a machine they use every day. A request path
//! with no memory can only accept absolute quantities, which is to say it can
//! only accept instructions phrased the way a form would ask for them.
//!
//! Deliberately small. It stores what was set and what the person said about
//! the result, because that is what a relative request refers to. It is not a
//! log, and it is not the audit trail: [`crate::record::DecisionRecord`] is
//! that, and it is kept separately because the two are read for different
//! reasons and would grow different shapes.

use std::collections::BTreeMap;

use adi_protocol::ids::VariableId;
use adi_protocol::units::Quantity;

use crate::cycle::SubjectiveFeedback;

/// One cycle, as it will be referred to later.
#[derive(Debug, Clone)]
pub struct Remembered {
    pub cycle_id: String,
    /// The style asked for, where one was.
    pub style: Option<String>,
    /// The settings in force. This is what a relative target resolves against.
    pub settings: BTreeMap<VariableId, Quantity>,
    /// What the person said about it afterwards.
    pub feedback: SubjectiveFeedback,
}

impl Remembered {
    pub fn setting(&self, variable: &VariableId) -> Option<Quantity> {
        self.settings.get(variable).copied()
    }
}

/// Somewhere to look up an earlier cycle.
///
/// A trait because on an appliance this is a small ring buffer in flash, in a
/// test it is a vector, and neither should be baked into the resolver.
pub trait Memory {
    /// The most recent cycle, if there has been one.
    fn last(&self) -> Option<&Remembered>;

    /// A specific cycle by id.
    fn get(&self, cycle_id: &str) -> Option<&Remembered>;

    /// Up to `n` most recent cycles, newest first. For a model deciding whether
    /// the last correction actually did anything.
    fn recent(&self, n: usize) -> Vec<&Remembered>;
}

/// A bounded in-memory store. Oldest entries fall off the end.
///
/// Bounded rather than growing, because the appliance version of this has a
/// fixed budget and a resolver that behaves differently on a host with infinite
/// history would not be testing the shipping behaviour.
#[derive(Debug, Clone)]
pub struct RecentCycles {
    entries: Vec<Remembered>,
    capacity: usize,
}

impl RecentCycles {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// Newest first.
    pub fn push(&mut self, entry: Remembered) {
        self.entries.insert(0, entry);
        self.entries.truncate(self.capacity);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for RecentCycles {
    fn default() -> Self {
        Self::new(32)
    }
}

impl Memory for RecentCycles {
    fn last(&self) -> Option<&Remembered> {
        self.entries.first()
    }

    fn get(&self, cycle_id: &str) -> Option<&Remembered> {
        self.entries.iter().find(|e| e.cycle_id == cycle_id)
    }

    fn recent(&self, n: usize) -> Vec<&Remembered> {
        self.entries.iter().take(n).collect()
    }
}

/// A machine that has run nothing yet. Every relative request against it is
/// refused for want of a reference, which is correct: the first time someone
/// asks for "less than last time", there was no last time.
pub struct NoMemory;

impl Memory for NoMemory {
    fn last(&self) -> Option<&Remembered> {
        None
    }
    fn get(&self, _: &str) -> Option<&Remembered> {
        None
    }
    fn recent(&self, _: usize) -> Vec<&Remembered> {
        Vec::new()
    }
}
