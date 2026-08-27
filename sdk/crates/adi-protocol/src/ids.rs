//! Semantic identifiers.
//!
//! A registry id fixes meaning across manufacturers. A vendor id is namespaced
//! and carries no agreed meaning until a skill response describes it.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Identifier for a tool, signal, event or procedure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SemanticId(pub String);

impl SemanticId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Registry ids begin `adi.`. Their meaning is fixed by the profile.
    pub fn is_registry(&self) -> bool {
        self.0.starts_with("adi.")
    }

    /// Vendor ids begin `x.`. See ADI-P section 8.
    pub fn is_vendor(&self) -> bool {
        self.0.starts_with("x.")
    }

    /// The namespace a vendor id belongs to, for example `x.example`.
    pub fn vendor_namespace(&self) -> Option<&str> {
        if !self.is_vendor() {
            return None;
        }
        let mut parts = self.0.splitn(3, '.');
        let x = parts.next()?;
        let vendor = parts.next()?;
        let _rest = parts.next()?;
        if x == "x" && !vendor.is_empty() {
            // Length of "x." plus the vendor segment.
            Some(&self.0[..2 + vendor.len()])
        } else {
            None
        }
    }

    pub fn is_well_formed(&self) -> bool {
        (self.is_registry() || self.is_vendor())
            && self.0.split('.').count() >= 3
            && !self.0.ends_with('.')
    }
}

impl fmt::Display for SemanticId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SemanticId {
    fn from(s: &str) -> Self {
        SemanticId(s.to_string())
    }
}

/// A domain variable the engine models, for example `shot_time` in the espresso
/// profile. Neutral crates treat these as opaque names; only a domain profile
/// knows what they mean.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VariableId(pub String);

impl VariableId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VariableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for VariableId {
    fn from(s: &str) -> Self {
        VariableId(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures use a fictional domain on purpose. A real domain's identifiers
    // in a neutral crate is the leak this crate exists to avoid.
    #[test]
    fn classifies_ids() {
        let registry = SemanticId::from("adi.example.actuator.start");
        assert!(registry.is_registry());
        assert!(!registry.is_vendor());
        assert!(registry.is_well_formed());

        let vendor = SemanticId::from("x.acme.custom.routine");
        assert!(vendor.is_vendor());
        assert_eq!(vendor.vendor_namespace(), Some("x.acme"));
    }

    #[test]
    fn rejects_malformed_ids() {
        assert!(!SemanticId::from("adi.example").is_well_formed());
        assert!(!SemanticId::from("nonsense.a.b").is_well_formed());
        assert!(!SemanticId::from("adi.example.trailing.").is_well_formed());
    }
}
