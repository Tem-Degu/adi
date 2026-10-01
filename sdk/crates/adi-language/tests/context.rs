//! The context budget.
//!
//! On an appliance-class processor the cost of a request is dominated by the
//! prompt. These tests exist so that the amount sent to a model is a decision
//! recorded here rather than an accident of whatever the memory happened to
//! hold.

use std::collections::BTreeMap;

use adi_core::{DecisionRecord, RecentCycles, Remembered, SelectedBy, SubjectiveFeedback};
use adi_decision::SelectionContext;
use adi_language::context::{estimate_tokens, ContextBudget, ModelContext};
use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::knowledge::{Knowledge, KnowledgeBase};
use adi_protocol::units::{Quantity, Unit};

fn knowledge() -> Knowledge {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/knowledge/neutral-base.yaml");
    let text = std::fs::read_to_string(&path).expect("fixture");
    let base: KnowledgeBase = serde_yaml::from_str(&text).expect("parses");
    Knowledge::new(base.into_layer()).expect("loads")
}

fn remembered(n: usize) -> RecentCycles {
    let mut memory = RecentCycles::new(64);
    for i in 0..n {
        let mut settings = BTreeMap::new();
        settings.insert(
            VariableId::from("position"),
            Quantity::new(20.0 + i as f64, Unit::Step),
        );
        settings.insert(
            VariableId::from("cycle_time"),
            Quantity::new(30.0, Unit::S),
        );
        memory.push(Remembered {
            cycle_id: format!("c{i}"),
            style: Some("adi.example.style.standard".into()),
            settings,
            feedback: SubjectiveFeedback::default(),
        });
    }
    memory
}

fn decisions(n: usize) -> Vec<DecisionRecord> {
    (0..n)
        .map(|i| DecisionRecord {
            cycle_id: format!("c{i}"),
            decision_id: format!("d{i}"),
            autonomy_level: "D2",
            cause: "off_target".into(),
            rule_id: String::new(),
            selected_tool: Some(SemanticId::from("adi.example.actuator.set_position")),
            selected_by: SelectedBy::Engine,
            inputs: Vec::new(),
            arguments: BTreeMap::new(),
            clamped_from: BTreeMap::new(),
            explanation: String::new(),
        })
        .collect()
}

fn variables() -> Vec<VariableId> {
    vec![
        VariableId::from("cycle_time"),
        VariableId::from("output_ratio"),
    ]
}

/// ADI-P 9.4.1 lists what an interpreter must receive. Omitting any of it means
/// asking a model to interpret "half of last time" without telling it what last
/// time was.
#[test]
fn the_model_is_given_what_the_protocol_requires() {
    let k = knowledge();
    let memory = remembered(2);
    let history = decisions(1);
    let context = SelectionContext {
        history: &history,
        cycle: None,
        memory: Some(&memory),
        rejected: None,
    };

    let assembled = ModelContext::assemble(
        "half of last time",
        &k,
        &variables(),
        &context,
        ContextBudget::default(),
    );

    assert_eq!(assembled.utterance, "half of last time");
    assert!(!assembled.styles.is_empty(), "the declared styles");
    assert!(!assembled.variables.is_empty(), "what it may name");
    assert_eq!(assembled.remembered.len(), 2, "what the machine produced");
    assert_eq!(assembled.history.len(), 1, "what was decided before");

    let rendered = assembled.render();
    assert!(rendered.contains("half of last time"));
    assert!(rendered.contains("position=20"), "settings a target resolves against");
}

#[test]
fn only_the_most_recent_cycles_are_offered() {
    let k = knowledge();
    let memory = remembered(20);
    let context = SelectionContext {
        history: &[],
        cycle: None,
        memory: Some(&memory),
        rejected: None,
    };

    let budget = ContextBudget {
        remembered_cycles: 3,
        ..ContextBudget::default()
    };
    let assembled = ModelContext::assemble("again", &k, &variables(), &context, budget);

    assert_eq!(assembled.remembered.len(), 3);
    assert_eq!(assembled.remembered[0].cycle_id, "c19", "newest first");
}

/// The test this module exists for. Without a bound, a machine that has run all
/// week sends a prompt that has grown all week, and the request that takes
/// twenty seconds is the thousandth one rather than the first.
#[test]
fn the_budget_is_enforced_however_much_is_remembered() {
    let k = knowledge();
    let memory = remembered(500);
    let history = decisions(500);
    let context = SelectionContext {
        history: &history,
        cycle: None,
        memory: Some(&memory),
        rejected: None,
    };

    let budget = ContextBudget {
        remembered_cycles: 3,
        history: 3,
        max_tokens: 256,
    };
    let assembled = ModelContext::assemble("again", &k, &variables(), &context, budget);

    assert!(
        assembled.estimated_tokens() <= budget.max_tokens,
        "assembled {} tokens against a budget of {}",
        assembled.estimated_tokens(),
        budget.max_tokens
    );
}

/// Truncation has an order, and the utterance is not in it. A request with the
/// request removed is not a smaller request.
#[test]
fn truncation_drops_history_before_it_drops_memory() {
    let k = knowledge();
    let memory = remembered(3);
    let history = decisions(3);
    let context = SelectionContext {
        history: &history,
        cycle: None,
        memory: Some(&memory),
        rejected: None,
    };

    // Tight enough to force a drop, loose enough that not everything goes.
    let budget = ContextBudget {
        remembered_cycles: 3,
        history: 3,
        max_tokens: 90,
    };
    let assembled = ModelContext::assemble("half of last time", &k, &variables(), &context, budget);

    assert_eq!(assembled.utterance, "half of last time", "never dropped");
    assert!(
        assembled.truncated.contains(&"history"),
        "history goes first, got {:?}",
        assembled.truncated
    );
    assert!(
        !assembled.remembered.is_empty(),
        "at least the last cycle survives, or a relative request cannot resolve"
    );
}

/// What was dropped is recorded. A model that answered badly because it was
/// given less than usual should be diagnosable afterwards.
#[test]
fn what_was_dropped_is_reported() {
    let k = knowledge();
    let memory = remembered(10);
    let history = decisions(10);
    let context = SelectionContext {
        history: &history,
        cycle: None,
        memory: Some(&memory),
        rejected: None,
    };

    let tight = ContextBudget {
        remembered_cycles: 5,
        history: 5,
        max_tokens: 60,
    };
    let assembled = ModelContext::assemble("again", &k, &variables(), &context, tight);
    assert!(!assembled.truncated.is_empty(), "truncation is not silent");

    let roomy = ContextBudget::default();
    let full = ModelContext::assemble("again", &k, &variables(), &context, roomy);
    assert!(full.truncated.is_empty(), "nothing dropped when it fits");
}

#[test]
fn an_empty_memory_is_not_an_error() {
    let k = knowledge();
    let context = SelectionContext {
        history: &[],
        cycle: None,
        memory: None,
        rejected: None,
    };

    let assembled = ModelContext::assemble(
        "make one",
        &k,
        &variables(),
        &context,
        ContextBudget::default(),
    );

    assert!(assembled.remembered.is_empty());
    assert!(assembled.history.is_empty());
    assert!(
        assembled.render().contains("make one"),
        "a first request still has a request in it"
    );
}

#[test]
fn the_token_estimate_is_monotonic_in_length() {
    assert_eq!(estimate_tokens(""), 0);
    assert!(estimate_tokens("a longer string") > estimate_tokens("short"));
}

/// The default budget is a claim about latency, so it is asserted rather than
/// left to drift. Changing it should be a deliberate act that fails a test.
#[test]
fn the_default_budget_is_what_was_decided() {
    let budget = ContextBudget::default();
    assert_eq!(budget.remembered_cycles, 3);
    assert_eq!(budget.history, 3);
    assert_eq!(budget.max_tokens, 1024);
}

/// Notes are dropped before anything a request depends on.
///
/// Reaching this means the notes do not fit the model the engine is configured
/// for, which is the author's fault rather than a runtime condition. The engine
/// degrades and says so rather than overflowing the model's context silently.
#[test]
fn notes_are_the_first_thing_a_tight_budget_drops() {
    let knowledge = knowledge();
    let mut context = ModelContext::assemble(
        "why",
        &knowledge,
        &[],
        &SelectionContext {
            history: &[],
            cycle: None,
            memory: None,
            rejected: None,
        },
        ContextBudget::default(),
    );

    context.notes = vec![
        adi_language::context::NoteLine { id: "a".into(), text: "x ".repeat(400) },
        adi_language::context::NoteLine { id: "b".into(), text: "y ".repeat(400) },
    ];
    context.styles = vec!["adi.example.style.one".into()];

    context.enforce(ContextBudget {
        remembered_cycles: 3,
        history: 2,
        max_tokens: 200,
    });

    assert!(
        context.truncated.contains(&"notes"),
        "notes should be dropped and reported, got {:?}",
        context.truncated
    );
    assert!(
        !context.styles.is_empty(),
        "and styles, which an intent needs, should survive"
    );
}
