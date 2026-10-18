//! The espresso domain profile.
//!
//! This is the only crate in the SDK permitted to know what coffee is.

pub mod registry;
pub mod rules;

use adi_core::cycle::Cycle;
use adi_core::domain::{Diagnosis, Domain};
use adi_protocol::ids::VariableId;
use adi_protocol::knowledge::{Knowledge, KnowledgeBase, KnowledgeExtension, Layer, ResolvedTargets};

/// The base knowledge, compiled in.
///
/// Embedded rather than read from disk because the engine runs on an appliance
/// module with no filesystem to read from, and because the knowledge a build
/// shipped with should be a property of that build rather than of whatever
/// happens to be next to the binary.
const BASE_KNOWLEDGE: &str = include_str!("../../../profiles/espresso/knowledge.yaml");

pub struct Espresso {
    variables: Vec<VariableId>,
    knowledge: Knowledge,
}

#[derive(Debug, thiserror::Error)]
pub enum EspressoError {
    #[error("the espresso knowledge base does not parse: {0}")]
    Parse(#[from] serde_yaml::Error),
    /// The extension is malformed in itself: bad namespace, redefinition,
    /// a missing parent. ADI-P 6.5 items 1 to 4 and 6.
    #[error(transparent)]
    Extension(#[from] adi_protocol::ExtensionError),
    /// The extension is well formed but does not layer: it widens a constraint,
    /// or changes a unit. ADI-P 6.4.
    #[error(transparent)]
    Knowledge(#[from] adi_protocol::KnowledgeError),
}

impl Default for Espresso {
    fn default() -> Self {
        Self::new()
    }
}

impl Espresso {
    /// Panics only if the compiled-in knowledge base is malformed, which is a
    /// build fault rather than a runtime one and is caught by the tests in this
    /// crate. `try_new` is available where a caller would rather handle it.
    pub fn new() -> Self {
        Self::try_new().expect("the compiled-in espresso knowledge base must be valid")
    }

    pub fn try_new() -> Result<Self, EspressoError> {
        let base: KnowledgeBase = serde_yaml::from_str(BASE_KNOWLEDGE)?;
        let knowledge = Knowledge::new(base.into_layer())?;

        // Read from the knowledge rather than held here. The two used to be
        // separate, a list in this file and a table in PROTOCOL.md 7.3, with
        // nothing checking they agreed.
        //
        // Taken through the same call `add_extension` uses, so that a profile
        // built from the base alone and one that has accepted a layer derive
        // this list the same way.
        let variables: Vec<VariableId> =
            knowledge.variable_ids().into_iter().cloned().collect();

        Ok(Self {
            variables,
            knowledge,
        })
    }

    /// Accept a manufacturer's or integrator's knowledge extension.
    ///
    /// Validated structurally first, then for constraint monotonicity as it is
    /// layered. A rejected extension leaves the profile exactly as it was, per
    /// ADI-P 6.5: the machine keeps working on the knowledge it already has.
    pub fn add_extension(
        &mut self,
        extension: KnowledgeExtension,
        layer: Layer,
    ) -> Result<(), EspressoError> {
        let base: KnowledgeBase = serde_yaml::from_str(BASE_KNOWLEDGE)?;
        extension.validate(&base)?;
        self.knowledge.add(extension.into_layer(layer))?;

        // An extension may introduce variables of its own (ADI-P 6.3), and
        // until this list is rebuilt they exist in the knowledge and are
        // invisible to `variables()`. The consequence is narrow and easy to
        // miss: the planner works from the skill's declared responses, so a
        // vendor variable is already actuable, while the interpreter context
        // is built from this list, so the same variable cannot be named in a
        // request. ADI-P 9.4.1 requires it be supplied.
        //
        // Rebuilt only after `add` has returned, so a rejected extension
        // leaves the profile as it was. Section 6.5 requires that, and
        // updating this list before the layer was accepted would have
        // reintroduced the partial state the rule exists to prevent.
        self.variables = self
            .knowledge
            .variable_ids()
            .into_iter()
            .cloned()
            .collect();
        Ok(())
    }

}

impl Domain for Espresso {
    fn profile_id(&self) -> &str {
        registry::PROFILE_ID
    }

    fn variables(&self) -> &[VariableId] {
        &self.variables
    }

    fn tool_description(&self, tool: &adi_protocol::ids::SemanticId) -> Option<&str> {
        registry::description(tool.as_str())
    }

    fn knowledge(&self) -> &Knowledge {
        &self.knowledge
    }

    fn derive(&self, cycle: &mut Cycle) {
        rules::derive(cycle);
    }

    fn diagnose(&self, cycle: &Cycle, targets: &ResolvedTargets) -> Diagnosis {
        rules::diagnose(cycle, targets)
    }

    /// Where each modelled variable was read from, for the decision record.
    ///
    /// Four of these come from the one mass curve, which is why the engine
    /// cannot infer this: `derive` reads a single trace and produces the yield,
    /// the shot time and both flow figures from it. The two settings are
    /// reported by the machine on their own signals.
    ///
    /// `extraction_yield` is absent deliberately. Nothing in this profile
    /// derives it from a signal, and naming one here would put a measurement in
    /// the record that was never taken.
    fn signal_for(&self, variable: &VariableId) -> Option<adi_protocol::ids::SemanticId> {
        use registry::{signals, vars};
        let signal = match variable.0.as_str() {
            vars::YIELD_MASS | vars::SHOT_TIME | vars::FLOW_PEAK | vars::FLOW_VARIANCE
            | vars::RATIO => signals::MASS,
            vars::GRIND_SETTING => signals::GRIND_SETTING,
            vars::DOSE_MASS => signals::DOSE_MASS,
            vars::BREW_TEMPERATURE => signals::GROUP_TEMP,
            _ => return None,
        };
        Some(adi_protocol::ids::SemanticId::from(signal))
    }

    fn feedback_vocabulary(&self) -> &[&str] {
        registry::FEEDBACK_VOCABULARY
    }

    /// The trait route to the inherent method below, so a caller holding a
    /// profile chosen at runtime can still offer a manufacturer their own
    /// knowledge. Behaviour is identical; only the error type differs, and
    /// both arms of `EspressoError` that can arise here convert into it.
    fn add_extension(
        &mut self,
        extension: KnowledgeExtension,
        layer: Layer,
    ) -> Result<(), adi_core::domain::ExtensionRejected> {
        match Espresso::add_extension(self, extension, layer) {
            Ok(()) => Ok(()),
            Err(EspressoError::Extension(e)) => Err(e.into()),
            Err(EspressoError::Knowledge(e)) => Err(e.into()),
            // The compiled-in base is parsed at build time and tested in this
            // crate, so a parse failure here is a broken build rather than a
            // bad extension. Reported as unsupported so the caller still
            // continues on the knowledge it holds, which 6.5 requires.
            Err(EspressoError::Parse(_)) => Err(
                adi_core::domain::ExtensionRejected::Unsupported(
                    registry::PROFILE_ID.to_string(),
                ),
            ),
        }
    }
}
