//! Knowledge extensions and layering. ADI-P section 6.
//!
//! Layers resolve highest first. A higher layer may add entries and narrow
//! ranges. It may never widen one, and it may never touch the safety envelope.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::VariableId;
use crate::units::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    /// StreetAI. The domain knowledge base.
    Base = 0,
    /// Manufacturer additions, namespaced.
    Vendor = 1,
    /// Integrator or region.
    Deployment = 2,
    /// Learned from this user.
    User = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeExtension {
    pub namespace: String,
    pub extension_version: String,
    pub extends: Extends,
    /// Variables this vendor introduces, which the base does not model.
    ///
    /// PROTOCOL.md 6.3 permits these. Each MUST sit in the vendor's namespace
    /// and MUST NOT redefine a base variable, checked in `validate`.
    ///
    /// This is what makes a genuinely new capability reachable. A tool that
    /// moves nothing the base models can be declared, and without a variable to
    /// name in the skill, the engine could never select it.
    #[serde(default)]
    pub variables: BTreeMap<VariableId, VariableDef>,
    #[serde(default)]
    pub styles: Vec<StyleEntry>,
    #[serde(default)]
    pub material_priors: Vec<MaterialPrior>,
    #[serde(default)]
    pub defaults: Vec<DefaultEntry>,
    /// Narrower bounds on variables the base already constrains.
    ///
    /// PROTOCOL.md 6.4 lets any layer narrow a range and none widen one, so a
    /// vendor that knows its machine cannot hold a temperature above 94 can say
    /// so. Optional, and absent in most extensions.
    #[serde(default)]
    pub constraints: BTreeMap<String, Range>,
    /// What this vendor wants the machine able to say. See [`Note`].
    #[serde(default)]
    pub notes: Vec<Note>,
}

/// What a domain variable is.
///
/// Before this existed, variables were declared nowhere. They appeared as keys
/// inside style targets, as string constants in a profile, and as a table in
/// PROTOCOL.md 7.3 that no code read. The unit was restated in every style that
/// targeted the variable, and nothing checked those restatements agreed.
///
/// The description is sent to a model, which is otherwise given bare names and
/// must infer from them what a request means. "Less bitter" maps onto shot_time
/// only because a model trained on general text knows about coffee, which is
/// not a mechanism a second domain can rely on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableDef {
    pub unit: crate::units::Unit,
    /// One line: what it measures, and which direction means what.
    pub description: String,
}

/// Something a manufacturer wants the machine to be able to say.
///
/// Free text. What belongs in it is the manufacturer's judgement, not this
/// protocol's: a basket size, a descaling interval, a warning about third-party
/// parts, anything they believe the model should know. The mechanism is
/// specified here and the content is not.
///
/// The rule that bounds it, and the reason it can be free text at all:
///
/// **A note may inform an answer. It may never determine an action.**
///
/// Notes reach the model, and an intent is still validated exactly as without
/// them: a closed schema, targets only on declared variables, every value
/// clamped to the declared range and checked against the envelope. So a note
/// cannot cause an undeclared action or an out-of-range value.
///
/// What a note must not contain is a target or a magnitude. "Aim for 27
/// seconds" belongs in a style and "moves shot time 1.4 s per step" belongs in
/// the skill, because those paths have validation and monotonicity rules that
/// prose does not. A number stated in two places will diverge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Namespaced for a vendor, as every other identifier is. Named in an
    /// answer's `drew_on`, so a record can say what the machine drew on.
    pub id: String,
    /// Words that should bring this note into context.
    ///
    /// Optional. A machine with a handful of notes needs none; one with sixty
    /// does, because the context budget cannot hold them all and the ones
    /// relevant to the question are the ones worth sending.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Optionally, the tool, variable or style this is about, so the note
    /// surfaces when that is under discussion without the person having used
    /// any of its keywords.
    #[serde(default)]
    pub about: Option<String>,
    pub text: String,
}

impl Note {
    /// Whether this note should be offered for an utterance.
    ///
    /// On the type rather than on `Knowledge`, because a caller that has
    /// precomputed its context, as the process backend does to avoid keeping
    /// the knowledge alive, needs the same rule and must not reimplement it.
    ///
    /// A note with neither keywords nor an `about` is always offered, which
    /// suits a machine with a handful of them. One with either is offered when
    /// it matches, which is what lets a machine carry many.
    pub fn matches(&self, utterance: &str) -> bool {
        if self.about.is_none() && self.keywords.is_empty() {
            return true;
        }

        let lowered = utterance.to_lowercase();
        let words: Vec<&str> = lowered
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();

        let mentions = |needle: &str| {
            let needle = needle.to_lowercase();
            words.contains(&needle.as_str()) || lowered.contains(&needle)
        };

        self.about.as_deref().is_some_and(&mentions)
            || self.keywords.iter().any(|k| mentions(k))
    }
}

/// The base domain knowledge, owned by StreetAI. Layer 0.
///
/// Structurally the same as an extension, minus the namespace and version
/// negotiation an extension needs, plus the things only the base may define:
/// hard constraints and the list of preference variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBase {
    pub profile: String,
    pub knowledge_version: String,
    /// The variables this profile models, with their units and meanings.
    ///
    /// Authoritative: a profile reads its variable list from here rather than
    /// holding its own, so the two cannot drift.
    #[serde(default)]
    pub variables: BTreeMap<VariableId, VariableDef>,
    #[serde(default)]
    pub styles: Vec<StyleEntry>,
    #[serde(default)]
    pub material_priors: Vec<MaterialPrior>,
    /// How far to move a variable when no skill response covers it.
    #[serde(default)]
    pub default_steps: BTreeMap<String, crate::units::Quantity>,
    #[serde(default)]
    pub constraints: BTreeMap<String, Range>,
    /// What the machine can tell a person, beyond what the structured data
    /// already says. See [`Note`].
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Variables that express taste rather than process. The engine must never
    /// try to correct a process fault by changing one. See PROTOCOL.md 7.1.
    #[serde(default)]
    pub preference_variables: Vec<VariableId>,
}

/// One layer of knowledge in the form resolution works with.
///
/// Base files and vendor extensions arrive in slightly different shapes,
/// because each carries fields the other has no use for. Both collapse to this,
/// so layering has one kind of thing to combine rather than a special case per
/// source.
#[derive(Debug, Clone)]
pub struct KnowledgeLayer {
    pub layer: Layer,
    /// Empty for the base, which owns the unprefixed namespace.
    pub namespace: Option<String>,
    pub variables: BTreeMap<VariableId, VariableDef>,
    pub notes: Vec<Note>,
    pub styles: Vec<StyleEntry>,
    pub material_priors: Vec<MaterialPrior>,
    pub default_steps: BTreeMap<String, crate::units::Quantity>,
    pub constraints: BTreeMap<String, Range>,
    pub preference_variables: Vec<VariableId>,
}

impl KnowledgeBase {
    pub fn into_layer(self) -> KnowledgeLayer {
        KnowledgeLayer {
            layer: Layer::Base,
            namespace: None,
            variables: self.variables,
            notes: self.notes,
            styles: self.styles,
            material_priors: self.material_priors,
            default_steps: self.default_steps,
            constraints: self.constraints,
            preference_variables: self.preference_variables,
        }
    }

    /// Every identifier this base introduces, for the redefinition check a
    /// higher layer has to pass.
    pub fn identifiers(&self) -> Vec<&str> {
        self.styles
            .iter()
            .map(|s| s.id.as_str())
            .chain(self.material_priors.iter().map(|m| m.id.as_str()))
            .collect()
    }
}

impl KnowledgeExtension {
    /// Collapse into a layer. `layer` says which, since the same document can
    /// be supplied by a manufacturer or by an integrator.
    pub fn into_layer(self, layer: Layer) -> KnowledgeLayer {
        let default_steps = self
            .defaults
            .iter()
            .map(|d| {
                (
                    d.variable.0.clone(),
                    crate::units::Quantity::new(d.value, d.unit),
                )
            })
            .collect();

        KnowledgeLayer {
            layer,
            namespace: Some(self.namespace),
            variables: self.variables,
            notes: self.notes,
            styles: self.styles,
            material_priors: self.material_priors,
            default_steps,
            constraints: self.constraints,
            // Only the base declares which variables are preferences. A vendor
            // that could add one could reclassify a process variable as taste
            // and take it out of the engine's reach.
            preference_variables: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extends {
    pub domain_profile: String,
    pub base_min_version: String,
}

/// A named outcome style with its target ranges. In espresso a style is a drink;
/// in another domain it is whatever the profile says it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleEntry {
    pub id: String,
    #[serde(default)]
    pub inherits: Option<String>,
    #[serde(default)]
    pub targets: BTreeMap<String, Range>,
}

/// Prior expectations for a class of input material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialPrior {
    pub id: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub offsets: BTreeMap<String, f64>,
    #[serde(default)]
    pub targets: BTreeMap<String, Range>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultEntry {
    pub variable: VariableId,
    pub value: f64,
    pub unit: crate::units::Unit,
    #[serde(default)]
    pub rationale_note_id: Option<String>,
}

/// A value together with the layer that produced it.
///
/// ADI-P 9.3 requires every decision to be reconstructable, and layer
/// attribution is what makes "the dose came from the vendor default, the
/// temperature from the user's learned preference" possible.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Attributed<T> {
    pub value: T,
    pub layer: Layer,
}

impl<T> Attributed<T> {
    pub fn new(value: T, layer: Layer) -> Self {
        Self { value, layer }
    }
}

/// Resolves a target across layers.
///
/// Constraint monotonicity, ADI-P 6.4: the effective range is the intersection
/// of every layer. Any layer that would widen the range is a validation error,
/// not a silent override.
#[derive(Debug, Clone, Default)]
pub struct ResolvedTargets {
    entries: BTreeMap<String, Attributed<Range>>,
}

impl ResolvedTargets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one layer's contribution. Returns an error if the layer widens an
    /// existing range or changes its unit.
    pub fn apply(
        &mut self,
        key: &str,
        range: Range,
        layer: Layer,
    ) -> Result<(), MonotonicityError> {
        match self.entries.get(key) {
            None => {
                self.entries
                    .insert(key.to_string(), Attributed::new(range, layer));
                Ok(())
            }
            Some(existing) => {
                if existing.value.unit != range.unit {
                    return Err(MonotonicityError::UnitMismatch {
                        key: key.to_string(),
                        layer,
                        expected: existing.value.unit,
                        found: range.unit,
                    });
                }
                // Disjoint or empty first, so it is not misreported as widening.
                let narrowed = existing.value.intersect(&range).ok_or_else(|| {
                    MonotonicityError::Incompatible {
                        key: key.to_string(),
                        layer,
                    }
                })?;
                // The test is on the incoming range, not the intersection.
                // Intersecting has already clamped to the narrower of the two,
                // so comparing the result against `existing` can never fail and
                // a widening layer would be accepted with its widening quietly
                // dropped. 6.4 requires the layer be rejected, not corrected.
                if range.min < existing.value.min || range.max > existing.value.max {
                    return Err(MonotonicityError::Widened {
                        key: key.to_string(),
                        layer,
                    });
                }
                // A layer that restates what already held has not narrowed
                // anything, so attribution stays with the layer that did.
                // Section 9.3 asks which layer produced the effective value,
                // not which one mentioned it last.
                if narrowed.min == existing.value.min && narrowed.max == existing.value.max {
                    return Ok(());
                }
                self.entries
                    .insert(key.to_string(), Attributed::new(narrowed, layer));
                Ok(())
            }
        }
    }

    /// Combine two opinions about what is wanted, keeping the overlap.
    ///
    /// Distinct from `apply`, and the difference is not a technicality.
    /// `apply` enforces 6.4: a layer may not widen a constraint, so anything
    /// reaching outside what already holds is a rejected layer. This is for
    /// composing preferences, where reaching outside is ordinary. A material
    /// prior saying "this batch runs long" against a style saying "an espresso
    /// is 1.8 to 2.4" means the overlap, not a fault.
    ///
    /// Disjoint is still an error: there is no value that satisfies both, and
    /// silently picking one would make the machine claim a target nobody set.
    pub fn narrow_to(
        &mut self,
        key: &str,
        range: Range,
        layer: Layer,
    ) -> Result<(), MonotonicityError> {
        let Some(existing) = self.entries.get(key) else {
            self.entries
                .insert(key.to_string(), Attributed::new(range, layer));
            return Ok(());
        };
        if existing.value.unit != range.unit {
            return Err(MonotonicityError::UnitMismatch {
                key: key.to_string(),
                layer,
                expected: existing.value.unit,
                found: range.unit,
            });
        }
        let overlap =
            existing
                .value
                .intersect(&range)
                .ok_or_else(|| MonotonicityError::Incompatible {
                    key: key.to_string(),
                    layer,
                })?;
        if overlap.min == existing.value.min && overlap.max == existing.value.max {
            return Ok(());
        }
        self.entries
            .insert(key.to_string(), Attributed::new(overlap, layer));
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&Attributed<Range>> {
        self.entries.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Attributed<Range>)> {
        self.entries.iter()
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum MonotonicityError {
    #[error("layer {layer:?} widens the range for '{key}'; higher layers may only narrow")]
    Widened { key: String, layer: Layer },
    #[error("layer {layer:?} declares an incompatible or empty range for '{key}'")]
    Incompatible { key: String, layer: Layer },
    /// Intersecting ranges of different units would produce a number that means
    /// nothing. `adi-safety` had this same hole in accumulation.
    #[error("layer {layer:?} declares '{key}' in {found:?}, but it is already {expected:?}")]
    UnitMismatch {
        key: String,
        layer: Layer,
        expected: crate::units::Unit,
        found: crate::units::Unit,
    },
}

// ------------------------------------------------------------- validation

/// Structural faults in a knowledge extension. PROTOCOL.md 6.5.
///
/// Monotonicity is not here: it needs the resolved base to compare against, so
/// it is reported by resolution rather than by this check.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExtensionError {
    #[error("extension extends profile '{found}', but this engine implements '{expected}'")]
    WrongProfile { expected: String, found: String },
    #[error("identifier '{id}' is outside the vendor namespace '{namespace}'")]
    NotNamespaced { id: String, namespace: String },
    #[error("a vendor extension may not redefine base identifier '{0}'")]
    Redefinition(String),
    #[error("'{child}' inherits from '{parent}', which does not exist")]
    UnknownParent { child: String, parent: String },
    #[error("inheritance cycle through '{0}'")]
    InheritanceCycle(String),
    #[error("range for '{0}' is empty: min exceeds max")]
    EmptyRange(String),
}

impl KnowledgeExtension {
    /// Check everything that can be checked without resolving against the base
    /// values, per PROTOCOL.md 6.5 items 1 to 4 and 6.
    ///
    /// `base` supplies the identifiers a vendor may inherit from but must not
    /// redefine.
    pub fn validate(&self, base: &KnowledgeBase) -> Result<(), ExtensionError> {
        if self.extends.domain_profile != base.profile {
            return Err(ExtensionError::WrongProfile {
                expected: base.profile.clone(),
                found: self.extends.domain_profile.clone(),
            });
        }

        let base_ids = base.identifiers();
        let prefix = format!("{}.", self.namespace);

        let mine: Vec<&str> = self
            .styles
            .iter()
            .map(|s| s.id.as_str())
            .chain(self.material_priors.iter().map(|m| m.id.as_str()))
            .collect();

        for note in &self.notes {
            if base.notes.iter().any(|n| n.id == note.id) {
                return Err(ExtensionError::Redefinition(note.id.clone()));
            }
            if !note.id.starts_with(&prefix) {
                return Err(ExtensionError::NotNamespaced {
                    id: note.id.clone(),
                    namespace: self.namespace.clone(),
                });
            }
        }

        for id in &mine {
            // Redefinition is checked before namespacing, because it is the
            // more specific complaint and the more useful message.
            if base_ids.contains(id) {
                return Err(ExtensionError::Redefinition((*id).to_string()));
            }
            if !id.starts_with(&prefix) {
                return Err(ExtensionError::NotNamespaced {
                    id: (*id).to_string(),
                    namespace: self.namespace.clone(),
                });
            }
        }

        // Every declared range must be satisfiable on its own terms. An empty
        // range would make every value illegal, which is never what anyone
        // meant to write.
        let ranges = self
            .styles
            .iter()
            .flat_map(|s| s.targets.iter())
            .chain(self.material_priors.iter().flat_map(|m| m.targets.iter()))
            .chain(self.constraints.iter());
        for (key, range) in ranges {
            if range.min > range.max {
                return Err(ExtensionError::EmptyRange(key.clone()));
            }
        }

        self.check_inheritance(base)
    }

    /// Every `inherits` target must exist, and the chain must terminate.
    ///
    /// Iterative with a step budget rather than recursive, so a cycle is
    /// reported instead of overflowing the stack.
    fn check_inheritance(&self, base: &KnowledgeBase) -> Result<(), ExtensionError> {
        let parent_of = |id: &str| -> Option<Option<String>> {
            self.styles
                .iter()
                .find(|s| s.id == id)
                .or_else(|| base.styles.iter().find(|s| s.id == id))
                .map(|s| s.inherits.clone())
        };

        for style in &self.styles {
            let mut current = style.inherits.clone();
            let budget = self.styles.len() + base.styles.len() + 1;

            for _ in 0..budget {
                let Some(parent) = current else {
                    break;
                };
                match parent_of(&parent) {
                    None => {
                        return Err(ExtensionError::UnknownParent {
                            child: style.id.clone(),
                            parent,
                        })
                    }
                    Some(next) => current = next,
                }
            }

            // Still walking after the budget means the chain does not end.
            if current.is_some() {
                return Err(ExtensionError::InheritanceCycle(style.id.clone()));
            }
        }

        Ok(())
    }
}

// ------------------------------------------------------------- resolution

/// The knowledge of one machine: a base plus whatever layers were accepted on
/// top of it. ADI-P section 6.
///
/// Two mechanisms live here, and they are deliberately not the same code path.
///
/// **Inheritance** (`inherits`) creates a new identifier that starts from
/// another's values and overrides the fields it names. It is override, not
/// intersection. The espresso base is the proof: `lungo` inherits from
/// `espresso` and sets a ratio of 3.0 to 4.0 against the parent's 1.8 to 2.4.
/// Those ranges do not overlap. Intersecting them would yield an empty range
/// and every lungo would fail to resolve.
///
/// **Layering** intersects: for one key, each higher layer may narrow the range
/// and none may widen it, per 6.4. The two never collide, because an identifier
/// is defined by exactly one layer; redefinition is refused before it gets here.
#[derive(Debug, Clone)]
pub struct Knowledge {
    layers: Vec<KnowledgeLayer>,
    /// Folded across layers as each is accepted, so a widening layer is caught
    /// when it is added rather than at the first resolution that trips over it.
    constraints: ResolvedTargets,
}

/// One resolved style, with the layer behind every effective value.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub style: String,
    pub material: Option<String>,
    pub targets: ResolvedTargets,
    /// Additive corrections from the material prior, for example a starting
    /// offset that puts a new batch near the mark instead of at the last one.
    pub offsets: BTreeMap<String, Attributed<f64>>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum KnowledgeError {
    #[error("knowledge must start from the base layer, but layer {0:?} was supplied")]
    NotBase(Layer),
    #[error("layer {0:?} is already present; each layer may be supplied once")]
    DuplicateLayer(Layer),
    #[error("layer {added:?} may not sit below {existing:?}")]
    OutOfOrder { added: Layer, existing: Layer },
    #[error("no style '{0}'")]
    UnknownStyle(String),
    #[error("no material '{0}'")]
    UnknownMaterial(String),
    #[error("'{child}' inherits from '{parent}', which does not exist")]
    UnknownParent { child: String, parent: String },
    #[error("inheritance cycle through '{0}'")]
    InheritanceCycle(String),
    #[error(transparent)]
    Monotonicity(#[from] MonotonicityError),
    #[error("style '{style}' targets '{variable}', which no layer declares")]
    UndeclaredVariable { style: String, variable: String },
    #[error("style '{style}' targets '{variable}' in {found}, but it is declared in {declared}")]
    TargetUnitMismatch {
        style: String,
        variable: String,
        declared: String,
        found: String,
    },
}

impl Knowledge {
    pub fn new(base: KnowledgeLayer) -> Result<Self, KnowledgeError> {
        if base.layer != Layer::Base {
            return Err(KnowledgeError::NotBase(base.layer));
        }
        let mut k = Self {
            layers: Vec::new(),
            constraints: ResolvedTargets::new(),
        };
        k.push(base)?;
        Ok(k)
    }

    /// Every note, across layers.
    ///
    /// For a caller that filters per utterance itself, which the process
    /// backend does to avoid keeping the knowledge alive for the session.
    pub fn notes(&self) -> impl Iterator<Item = &Note> {
        self.layers.iter().flat_map(|l| l.notes.iter())
    }

    /// Notes relevant to an utterance, in layer order.
    ///
    /// Selection is deliberately simple, because what belongs in a note is the
    /// manufacturer's concern and the mechanism is this protocol's. See
    /// [`Note::matches`].
    ///
    /// The caller applies the context budget. A machine with more notes than
    /// fit has them truncated and the context says so.
    pub fn notes_for(&self, utterance: &str) -> Vec<&Note> {
        self.layers
            .iter()
            .flat_map(|l| l.notes.iter())
            .filter(|n| n.matches(utterance))
            .collect()
    }

    /// What a variable is, from whichever layer declares it.
    ///
    /// Searched from the top down, though a higher layer cannot redefine a base
    /// variable, so at most one layer declares any given one.
    pub fn variable(&self, id: &VariableId) -> Option<&VariableDef> {
        self.layers.iter().rev().find_map(|l| l.variables.get(id))
    }

    /// Accept a layer, or reject it whole.
    ///
    /// Section 6.5 is explicit that a failing extension is rejected with a
    /// reason and the engine continues on the knowledge it already has. So this
    /// is atomic: on error nothing is retained, and the machine keeps working
    /// with slightly less knowledge rather than stopping. A vendor shipping a
    /// bad extension should not brick an appliance.
    pub fn add(&mut self, layer: KnowledgeLayer) -> Result<(), KnowledgeError> {
        if layer.layer == Layer::Base {
            return Err(KnowledgeError::DuplicateLayer(Layer::Base));
        }
        if let Some(top) = self.layers.last() {
            if layer.layer == top.layer {
                return Err(KnowledgeError::DuplicateLayer(layer.layer));
            }
            if layer.layer < top.layer {
                return Err(KnowledgeError::OutOfOrder {
                    added: layer.layer,
                    existing: top.layer,
                });
            }
        }
        self.push(layer)
    }

    fn push(&mut self, layer: KnowledgeLayer) -> Result<(), KnowledgeError> {
        self.check_targets_against_variables(&layer)?;

        // Fold into a copy first. If any constraint widens, the copy is dropped
        // and this instance is untouched.
        let mut trial = self.constraints.clone();
        for (key, range) in &layer.constraints {
            trial.apply(key, range.clone(), layer.layer)?;
        }
        self.constraints = trial;
        self.layers.push(layer);
        Ok(())
    }

    /// Every style target must name a declared variable, in that variable's
    /// unit.
    ///
    /// Nothing checked this before variables were declared, because there was
    /// nothing to check against: a target carried its own unit and a typo in a
    /// variable name produced a target that silently matched nothing. A unit
    /// disagreement was worse, since a range in the wrong unit compares against
    /// a measurement and quietly decides a shot was fine.
    fn check_targets_against_variables(
        &self,
        layer: &KnowledgeLayer,
    ) -> Result<(), KnowledgeError> {
        for style in &layer.styles {
            for (variable, range) in &style.targets {
                let id = VariableId::from(variable.as_str());

                // Declared by this layer, or by one already accepted. A vendor
                // style may target a base variable, which is the common case.
                let declared = layer
                    .variables
                    .get(&id)
                    .or_else(|| self.layers.iter().find_map(|l| l.variables.get(&id)));

                let Some(definition) = declared else {
                    return Err(KnowledgeError::UndeclaredVariable {
                        style: style.id.clone(),
                        variable: variable.clone(),
                    });
                };

                if range.unit != definition.unit {
                    return Err(KnowledgeError::TargetUnitMismatch {
                        style: style.id.clone(),
                        variable: variable.clone(),
                        declared: format!("{:?}", definition.unit),
                        found: format!("{:?}", range.unit),
                    });
                }
            }
        }
        Ok(())
    }

    /// Resolve a style, optionally through a material prior.
    ///
    /// Three composition rules meet here, and they are different on purpose.
    ///
    /// 1. **Inheritance** overrides. A child style replaces the fields it names
    ///    and inherits the rest, so it may sit anywhere, including outside its
    ///    parent's range.
    /// 2. **A style against the constraints** must be contained. Constraints
    ///    are the envelope, and a style reaching outside one is a fault in the
    ///    knowledge, reported rather than clamped: clamping would let a machine
    ///    claim a target nobody set.
    /// 3. **A material prior against a style** takes the overlap. A prior is a
    ///    second opinion about this batch rather than a bound on the style, so
    ///    leaning outside it is ordinary. Disjoint is still an error.
    pub fn resolve(
        &self,
        style_id: &str,
        material_id: Option<&str>,
    ) -> Result<Resolution, KnowledgeError> {
        let chain = self.inheritance_chain(style_id)?;

        // Inheritance: plain override, root first so a descendant wins.
        let mut inherited: BTreeMap<String, (Range, Layer)> = BTreeMap::new();
        for (style, layer) in chain.iter().rev() {
            for (key, range) in &style.targets {
                inherited.insert(key.clone(), (range.clone(), *layer));
            }
        }

        // Layering: intersection, broad to narrow.
        let mut targets = self.constraints.clone();
        for (key, (range, layer)) in inherited {
            targets.apply(&key, range, layer)?;
        }

        let mut offsets = BTreeMap::new();
        if let Some(id) = material_id {
            let (prior, layer) = self
                .material(id)
                .ok_or_else(|| KnowledgeError::UnknownMaterial(id.to_string()))?;
            // Composition, not layering: see `narrow_to`. A prior that leans
            // outside the style is normal and resolves to the overlap.
            for (key, range) in &prior.targets {
                targets.narrow_to(key, range.clone(), layer)?;
            }
            for (key, value) in &prior.offsets {
                offsets.insert(key.clone(), Attributed::new(*value, layer));
            }
        }

        Ok(Resolution {
            style: style_id.to_string(),
            material: material_id.map(str::to_string),
            targets,
            offsets,
        })
    }

    /// The style and its ancestors, nearest first. Iterative with a budget, so a
    /// cycle is reported rather than overflowing the stack.
    fn inheritance_chain(
        &self,
        style_id: &str,
    ) -> Result<Vec<(&StyleEntry, Layer)>, KnowledgeError> {
        let mut chain = Vec::new();
        let mut current = Some(style_id.to_string());
        let budget = self.layers.iter().map(|l| l.styles.len()).sum::<usize>() + 1;

        for _ in 0..budget {
            let Some(id) = current.take() else {
                return Ok(chain);
            };
            let Some((style, layer)) = self.style(&id) else {
                // The first lookup is the caller's identifier; any later one is
                // a broken `inherits`, and the two deserve different messages.
                return Err(match chain.last() {
                    None => KnowledgeError::UnknownStyle(id),
                    Some((child, _)) => KnowledgeError::UnknownParent {
                        child: child.id.clone(),
                        parent: id,
                    },
                });
            };
            chain.push((style, layer));
            current = style.inherits.clone();
        }

        Err(KnowledgeError::InheritanceCycle(style_id.to_string()))
    }

    fn style(&self, id: &str) -> Option<(&StyleEntry, Layer)> {
        self.layers
            .iter()
            .find_map(|l| l.styles.iter().find(|s| s.id == id).map(|s| (s, l.layer)))
    }

    fn material(&self, id: &str) -> Option<(&MaterialPrior, Layer)> {
        self.layers.iter().find_map(|l| {
            l.material_priors
                .iter()
                .find(|m| m.id == id)
                .map(|m| (m, l.layer))
        })
    }

    /// How far to move a variable when no skill response covers it. A point
    /// value, not a range, so the highest layer that names it simply wins.
    pub fn default_step(&self, variable: &str) -> Option<Attributed<crate::units::Quantity>> {
        self.layers
            .iter()
            .rev()
            .find_map(|l| l.default_steps.get(variable).map(|q| Attributed::new(*q, l.layer)))
    }

    /// The effective hard constraint, already intersected across every layer.
    pub fn constraint(&self, variable: &str) -> Option<&Attributed<Range>> {
        self.constraints.get(variable)
    }

    /// Whether a variable expresses taste rather than process. The engine must
    /// never try to correct a process fault by changing one. Base only, so a
    /// vendor cannot move a process variable out of the engine's reach.
    pub fn is_preference(&self, variable: &str) -> bool {
        self.layers
            .first()
            .is_some_and(|b| b.preference_variables.iter().any(|v| v.0 == variable))
    }

    /// Every style the machine can be asked for, across all layers.
    pub fn style_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .layers
            .iter()
            .flat_map(|l| l.styles.iter().map(|s| s.id.as_str()))
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Every material the machine knows a prior for, across all layers.
    ///
    /// ADI-P 9.4.1 requires an interpreter be given the declared styles and
    /// materials. A model that cannot name the material in front of it cannot
    /// express a request that depends on it.
    pub fn material_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .layers
            .iter()
            .flat_map(|l| l.material_priors.iter().map(|m| m.id.as_str()))
            .collect();
        ids.sort_unstable();
        ids
    }

    pub fn layers(&self) -> &[KnowledgeLayer] {
        &self.layers
    }
}
