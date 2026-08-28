//! Audio, and what a speech model made of it.
//!
//! Deliberately plain: signed 16-bit mono samples and a stated rate. That is
//! what a microphone on an appliance produces and what every speech backend
//! worth using accepts.

/// A block of recorded or synthesised audio.
///
/// The sample rate travels with the samples for the same reason every quantity
/// in this protocol travels with its unit. A buffer of numbers means nothing
/// without knowing how fast to read it, and the failure is not obvious: audio
/// at the wrong rate does not error, it just transcribes badly and the model
/// takes the blame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audio {
    /// Signed 16-bit mono PCM.
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

impl Audio {
    pub fn new(samples: Vec<i16>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
        }
    }

    pub fn duration_s(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f64 / f64::from(self.sample_rate)
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// What a speech model heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    /// 0 to 1. A transcript is a guess, and this says how much of one.
    pub confidence: f64,
}

impl Transcript {
    pub fn new(text: impl Into<String>, confidence: f64) -> Self {
        Self {
            text: text.into(),
            confidence,
        }
    }

    /// Whether this is worth acting on.
    ///
    /// A non-finite confidence is not reliable. A backend that cannot say how
    /// sure it is has not answered the question, and treating that as certainty
    /// would make the check worse than useless.
    pub fn is_reliable(&self, threshold: f64) -> bool {
        self.confidence.is_finite()
            && self.confidence >= threshold
            && !self.text.trim().is_empty()
    }
}
