//! The four model seams.
//!
//! The point of these is that the pipeline is assembled and correct before any
//! model exists, so slotting one in later is implementing a trait rather than
//! restructuring anything.

use adi_core::record::{DecisionRecord, SelectedBy};
use adi_decision::{DecisionError, Interpreter, SelectionContext, Understood};
use adi_language::{
    Audio, Explainer, Language, LanguageError, NoSpeaker, NoTranscriber, RecordExplainer, Speaker,
    Transcriber, Transcript,
};
use adi_protocol::intent::{Intent, IntentKind};
use std::collections::BTreeMap;

fn record(explanation: &str) -> DecisionRecord {
    DecisionRecord {
        cycle_id: "c1".into(),
        decision_id: "d1".into(),
        autonomy_level: "D2",
        cause: "below_target".into(),
        rule_id: "rule.example.slow".into(),
        selected_tool: None,
        selected_by: SelectedBy::Engine,
        inputs: Vec::new(),
        arguments: BTreeMap::new(),
        clamped_from: BTreeMap::new(),
        explanation: explanation.into(),
    }
}

fn audio(rate: u32) -> Audio {
    Audio::new(vec![0i16; 1600], rate)
}

// -------------------------------------------------------------- placeholders

/// A build with no models assembles and runs. It refuses what it cannot do and
/// says which part is missing, rather than returning silence or empty text.
#[test]
fn a_build_with_no_models_assembles_and_refuses_clearly() {
    let language = Language::placeholder();

    let err = language
        .understand(&audio(16_000), &SelectionContext::default())
        .expect_err("nothing can be heard without a speech model");
    assert!(
        matches!(err, LanguageError::NoBackend("speech to text")),
        "the error must name the missing seam, not just say something failed: {err}"
    );

    assert!(matches!(
        NoTranscriber.transcribe(&audio(16_000)),
        Err(LanguageError::NoBackend("speech to text"))
    ));
    assert!(matches!(
        NoSpeaker.speak("anything"),
        Err(LanguageError::NoBackend("text to speech"))
    ));
}

/// The explainer is the one seam that works without a model, because the domain
/// rules already write an explanation when they diagnose. Stilted, but true.
#[test]
fn a_build_with_no_models_can_still_explain() {
    let language = Language::placeholder();
    let (text, spoken) = language
        .report(&record("That ran 22 seconds against a target of 25 to 32."))
        .expect("explaining needs no model");

    assert!(text.contains("22 seconds"));
    assert!(spoken.is_none(), "no voice, but the words survive");
}

#[test]
fn a_decision_with_no_written_explanation_still_produces_a_sentence() {
    let text = RecordExplainer.explain(&record("")).unwrap();
    assert!(text.contains("below_target"));
    assert!(!text.is_empty());
}

// ------------------------------------------------------------------- audio

/// The sample rate travels with the samples for the same reason every quantity
/// travels with its unit. Audio at the wrong rate does not error on its own, it
/// transcribes badly, and the model takes the blame.
#[test]
fn audio_at_the_wrong_rate_is_refused_rather_than_resampled() {
    let language = Language::placeholder();
    let err = language
        .understand(&audio(44_100), &SelectionContext::default())
        .expect_err("44.1 kHz into a 16 kHz backend");
    assert!(matches!(
        err,
        LanguageError::SampleRate {
            expected: 16_000,
            found: 44_100
        }
    ));
}

#[test]
fn empty_audio_is_refused() {
    let language = Language::placeholder();
    assert!(matches!(
        language.understand(&Audio::new(Vec::new(), 16_000), &SelectionContext::default()),
        Err(LanguageError::NoAudio)
    ));
}

#[test]
fn audio_reports_its_own_duration() {
    assert_eq!(Audio::new(vec![0; 16_000], 16_000).duration_s(), 1.0);
    assert_eq!(Audio::new(vec![0; 100], 0).duration_s(), 0.0);
}

// ------------------------------------------------- hearing badly, and asking

struct Heard(&'static str, f64);

impl Transcriber for Heard {
    fn required_sample_rate(&self) -> u32 {
        16_000
    }
    fn transcribe(&self, _: &Audio) -> Result<Transcript, LanguageError> {
        Ok(Transcript::new(self.0, self.1))
    }
}

struct AlwaysProduce;

impl Interpreter for AlwaysProduce {
    fn interpret(&self, _: &str, _: &SelectionContext<'_>) -> Result<Understood, DecisionError> {
        Ok(Understood::Intent(Intent {
            kind: IntentKind::Produce,
            style: Some("adi.example.style.standard".into()),
            material: None,
            targets: BTreeMap::new(),
        }))
    }
}

fn with_transcriber(heard: Heard) -> Language<Heard, AlwaysProduce, RecordExplainer, NoSpeaker> {
    Language::new(heard, AlwaysProduce, RecordExplainer, NoSpeaker)
}

/// The transcript is the one place where a wrong word is indistinguishable from
/// a right one downstream. "Hotter" misheard as "shorter" is a physical action
/// on a machine with a heating element, so a transcript the backend is unsure
/// of becomes a question rather than an intent.
#[test]
fn a_transcript_heard_badly_becomes_a_question() {
    let language = with_transcriber(Heard("make it hotter", 0.3));
    let understood = language
        .understand(&audio(16_000), &SelectionContext::default())
        .expect("asking is not an error");

    match understood {
        Understood::Clarify { question, because } => {
            assert!(question.ends_with('?'));
            assert!(because.contains("confidence"), "the record says why: {because}");
        }
        other => panic!("expected a question, got {other:?}"),
    }
}

#[test]
fn a_transcript_heard_well_becomes_an_intent() {
    let language = with_transcriber(Heard("make me the usual", 0.95));
    assert!(matches!(
        language.understand(&audio(16_000), &SelectionContext::default()),
        Ok(Understood::Intent(_))
    ));
}

/// A backend that cannot say how sure it is has not answered the question.
/// Treating that as certainty would make the check worse than useless.
#[test]
fn a_confidence_that_is_not_a_number_is_not_reliable() {
    assert!(!Transcript::new("something", f64::NAN).is_reliable(0.6));
    assert!(!Transcript::new("", 1.0).is_reliable(0.6), "silence is not speech");
    assert!(!Transcript::new("   ", 1.0).is_reliable(0.6));
    assert!(Transcript::new("something", 0.6).is_reliable(0.6));
}

/// The threshold is deliberately not near-certain: a kitchen over a grinder is
/// not a quiet-room benchmark, and asking again costs a second against a wasted
/// drink. But it must be reachable, or the machine only ever asks.
#[test]
fn the_default_threshold_is_usable() {
    let t = adi_language::DEFAULT_MIN_TRANSCRIPT_CONFIDENCE;
    assert!(t > 0.0 && t < 1.0);
    assert!(Transcript::new("make me the usual", 0.8).is_reliable(t));
}

/// The person is told when a correction could not be carried out in full.
///
/// This is the whole point of recording a limit. The explainer reads the record
/// and nothing else, so before the record carried this, a machine whose
/// mechanism was already at the end of its travel reported having moved it.
#[test]
fn a_partial_correction_is_explained_as_partial() {
    let mut record = record("Reduced the feed rate.");
    record.arguments = BTreeMap::from([(
        "setting".to_string(),
        adi_protocol::units::Quantity::new(0.0, adi_protocol::units::Unit::Step),
    )]);
    record.clamped_from = BTreeMap::from([("setting".to_string(), -4.0)]);

    let text = RecordExplainer.explain(&record).unwrap();

    assert!(text.contains("Reduced the feed rate."), "the action is still stated: {text}");
    assert!(text.contains("partial"), "and the shortfall is stated: {text}");
    assert!(text.contains("-4"), "with what was wanted: {text}");
}

/// An ordinary correction gains nothing. A machine that appended a caveat to
/// every explanation would train people to ignore it.
#[test]
fn an_ordinary_correction_is_explained_without_a_caveat() {
    let record = record("Reduced the feed rate.");

    let text = RecordExplainer.explain(&record).unwrap();

    assert_eq!(text, "Reduced the feed rate.");
}
