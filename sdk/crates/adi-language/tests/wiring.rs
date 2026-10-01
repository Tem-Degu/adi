//! Compiles the example from sdk/README.md "Wiring a microphone and a speaker",
//! so the code on that page is known to type-check rather than assumed to.
use adi_decision::SelectionContext;
use adi_language::{Audio, Language, NoSpeaker, NoTranscriber, RecordExplainer};
use adi_decision::NoInterpreter;

#[test]
fn the_documented_wiring_compiles() {
    let transcriber = NoTranscriber;
    let interpreter = NoInterpreter;
    let explainer = RecordExplainer;
    let speaker = NoSpeaker;

    let language = Language::new(transcriber, interpreter, explainer, speaker);

    let samples: Vec<i16> = vec![0; 16];
    let audio = Audio::new(samples, 16_000);
    let context = SelectionContext {
        history: &[],
        cycle: None,
        memory: None,
        rejected: None,
    };

    // Both calls are expected to refuse here, because the seams are the
    // placeholders. What is being checked is that the shapes are right.
    let _ = language.understand(&audio, &context);

    use adi_language::Transcriber;
    let _rate = language.transcriber.required_sample_rate();

    // The other half: a decision becomes words and, where a speaker is
    // configured, samples. With NoSpeaker the words still come back, which is
    // what the documentation promises a build with no synthesis.
    let record = adi_core::DecisionRecord {
        cycle_id: "c1".into(),
        decision_id: "d1".into(),
        autonomy_level: "D0",
        cause: "cycle_time".into(),
        rule_id: String::new(),
        selected_tool: None,
        selected_by: adi_core::SelectedBy::Engine,
        inputs: Vec::new(),
        arguments: std::collections::BTreeMap::new(),
        clamped_from: std::collections::BTreeMap::new(),
        explanation: "Set the feed rate to 20.".into(),
    };

    let (text, spoken) = language.report(&record).expect("words without a voice");
    assert_eq!(text, "Set the feed rate to 20.");
    assert!(spoken.is_none(), "no speaker configured, so no samples");
}
