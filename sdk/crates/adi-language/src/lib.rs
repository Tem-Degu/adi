//! The language layer: where models attach, and nothing else.
//!
//! ```text
//!   audio in  -> [ Transcriber ]  -> words
//!                                     |
//!                [ Interpreter ] <---+          (in adi-decision)
//!                      |
//!                   intent  ==========>  the engine: resolve, plan, clamp,
//!                                        check the envelope, record
//!                                              |
//!                  words <- [ Explainer ] <-- decision
//!                    |
//!   audio out <- [ Speaker ]
//! ```
//!
//! Four seams, and every one of them is a model. This crate holds the traits
//! and a working non-model implementation of each, so the whole pipeline can be
//! assembled, wired to the engine and tested before any model exists. Dropping
//! in whisper.cpp, a local language model and piper later is implementing four
//! traits, not restructuring anything.
//!
//! Nothing here produces an actuation value. That is the whole reason the
//! language layer can be as capable as the best available model without
//! weakening the argument in ADI-P 9.3: the engine's job begins where this
//! crate's output ends, and it validates everything it is handed.
//!
//! Domain-neutral, like the rest of the engine side. Speech is speech.

use adi_core::record::DecisionRecord;
use adi_decision::{DecisionError, SelectionContext};

pub mod audio;
pub mod context;

pub use audio::{Audio, Transcript};
// Re-exported so the whole language layer is discoverable in one place, even
// though the intent seam lives next to the context type it needs.
pub use adi_decision::{Interpreter, NoInterpreter, Understood};

#[derive(Debug, Clone, thiserror::Error)]
pub enum LanguageError {
    #[error("no {0} backend is configured on this build")]
    NoBackend(&'static str),
    #[error("the audio is {found} Hz, but the backend requires {expected} Hz")]
    SampleRate { expected: u32, found: u32 },
    #[error("the audio is empty")]
    NoAudio,
    #[error("{0}")]
    Backend(String),
}

impl From<LanguageError> for DecisionError {
    fn from(e: LanguageError) -> Self {
        match e {
            LanguageError::NoBackend(what) => DecisionError::MalformedSelection(format!(
                "no {what} backend"
            )),
            other => DecisionError::MalformedSelection(other.to_string()),
        }
    }
}

// ----------------------------------------------------------------- listening

/// Turn recorded audio into words. A speech model.
///
/// The result carries a confidence, because a transcript is a guess and acting
/// on a bad one is how "make it hotter" becomes "make it shorter". See
/// [`Transcript::is_reliable`].
pub trait Transcriber {
    /// The sample rate this backend requires. Audio at any other rate is
    /// refused rather than resampled silently, because resampling in the wrong
    /// place is a subtle way to degrade recognition and blame the model.
    fn required_sample_rate(&self) -> u32;

    fn transcribe(&self, audio: &Audio) -> Result<Transcript, LanguageError>;
}

/// The transcriber on a build with no speech model.
pub struct NoTranscriber;

impl Transcriber for NoTranscriber {
    fn required_sample_rate(&self) -> u32 {
        16_000
    }

    fn transcribe(&self, _: &Audio) -> Result<Transcript, LanguageError> {
        Err(LanguageError::NoBackend("speech to text"))
    }
}

// ----------------------------------------------------------------- speaking

/// Turn words into audio. A speech synthesis model.
pub trait Speaker {
    fn sample_rate(&self) -> u32;
    fn speak(&self, text: &str) -> Result<Audio, LanguageError>;
}

/// The speaker on a build with no synthesis model.
///
/// A machine in this state is not mute: everything the [`Explainer`] produces is
/// text, and a screen or a log can show it. Only the voice is missing.
pub struct NoSpeaker;

impl Speaker for NoSpeaker {
    fn sample_rate(&self) -> u32 {
        22_050
    }
    fn speak(&self, _: &str) -> Result<Audio, LanguageError> {
        Err(LanguageError::NoBackend("text to speech"))
    }
}

// --------------------------------------------------------------- explaining

/// Turn a decision into something a person believes.
///
/// The record already holds every input, the layer behind each value and the
/// rule that fired. Turning that into a sentence is the difference between a
/// machine that adjusts itself and one that teaches you.
pub trait Explainer {
    fn explain(&self, record: &DecisionRecord) -> Result<String, LanguageError>;
}

/// Explains from the record alone, with no model.
///
/// Unlike the other three placeholders this one works, because the domain rules
/// already write an explanation when they diagnose. It is stilted and it cannot
/// answer a follow-up, but it is true, and a machine that says something true
/// and flat is better than one that says nothing.
pub struct RecordExplainer;

impl Explainer for RecordExplainer {
    fn explain(&self, record: &DecisionRecord) -> Result<String, LanguageError> {
        if record.explanation.is_empty() {
            return Ok(format!(
                "Diagnosed {} and made no change.",
                record.cause
            ));
        }
        Ok(record.explanation.clone())
    }
}

// ----------------------------------------------------------------- assembly

/// The four seams, assembled.
///
/// Holds the pipeline so an integrator swaps a backend in one place. A build
/// with no models at all is constructible and behaves correctly: it refuses to
/// listen and to speak, and still explains.
pub struct Language<T, P, E, S>
where
    T: Transcriber,
    P: Interpreter,
    E: Explainer,
    S: Speaker,
{
    pub transcriber: T,
    pub interpreter: P,
    pub explainer: E,
    pub speaker: S,
    /// Below this, a transcript is treated as not heard properly and the person
    /// is asked again rather than acted upon. See ADI-P 9.4.3.
    pub min_transcript_confidence: f64,
}

impl Language<NoTranscriber, NoInterpreter, RecordExplainer, NoSpeaker> {
    /// A build with no models. Assembles, runs, refuses the parts it cannot do.
    pub fn placeholder() -> Self {
        Self {
            transcriber: NoTranscriber,
            interpreter: NoInterpreter,
            explainer: RecordExplainer,
            speaker: NoSpeaker,
            min_transcript_confidence: DEFAULT_MIN_TRANSCRIPT_CONFIDENCE,
        }
    }
}

/// Deliberately not near-certain. Speech recognition in a kitchen, over a
/// grinder, is not a quiet-room benchmark, and the cost of asking again is a
/// second of someone's time against a wasted drink.
pub const DEFAULT_MIN_TRANSCRIPT_CONFIDENCE: f64 = 0.6;

impl<T, P, E, S> Language<T, P, E, S>
where
    T: Transcriber,
    P: Interpreter,
    E: Explainer,
    S: Speaker,
{
    pub fn new(transcriber: T, interpreter: P, explainer: E, speaker: S) -> Self {
        Self {
            transcriber,
            interpreter,
            explainer,
            speaker,
            min_transcript_confidence: DEFAULT_MIN_TRANSCRIPT_CONFIDENCE,
        }
    }

    /// Audio to intent, in one step.
    ///
    /// A transcript the backend is not confident in becomes a question rather
    /// than an intent. Mishearing "hotter" as "shorter" is a physical action on
    /// a machine with a heating element, and the transcript is the one place
    /// where a wrong word is indistinguishable from a right one downstream.
    pub fn understand(
        &self,
        audio: &Audio,
        context: &SelectionContext<'_>,
    ) -> Result<Understood, LanguageError> {
        if audio.samples.is_empty() {
            return Err(LanguageError::NoAudio);
        }
        let required = self.transcriber.required_sample_rate();
        if audio.sample_rate != required {
            return Err(LanguageError::SampleRate {
                expected: required,
                found: audio.sample_rate,
            });
        }

        let transcript = self.transcriber.transcribe(audio)?;
        if !transcript.is_reliable(self.min_transcript_confidence) {
            return Ok(Understood::Clarify {
                question: "Sorry, I did not catch that. Could you say it again?".into(),
                because: format!(
                    "transcript confidence {:.2} is below {:.2}",
                    transcript.confidence, self.min_transcript_confidence
                ),
            });
        }

        self.interpreter
            .interpret(&transcript.text, context)
            .map_err(|e| LanguageError::Backend(e.to_string()))
    }

    /// A decision, as words and then as sound.
    ///
    /// The words are returned alongside the audio, so a build with no synthesis
    /// still has something to show, and so what was said is recordable.
    pub fn report(&self, record: &DecisionRecord) -> Result<(String, Option<Audio>), LanguageError> {
        let text = self.explainer.explain(record)?;
        let spoken = match self.speaker.speak(&text) {
            Ok(audio) => Some(audio),
            // A missing voice is not a failure to explain.
            Err(LanguageError::NoBackend(_)) => None,
            Err(other) => return Err(other),
        };
        Ok((text, spoken))
    }
}
