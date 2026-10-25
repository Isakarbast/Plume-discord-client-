//! Opus encoding/decoding and the "is the user talking?" gate.

use opus::{Application, Bitrate, Channels, Decoder, Encoder};

use crate::VoiceError;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// Samples per channel in one 20 ms frame.
pub const FRAME_SAMPLES: usize = 960;
/// Interleaved stereo samples in one frame.
pub const FRAME_LEN: usize = FRAME_SAMPLES * CHANNELS;
/// What Discord sends between bursts of speech.
pub const SILENCE_FRAME: [u8; 3] = [0xF8, 0xFF, 0xFE];

/// The biggest frame Opus can produce (120 ms), for decode buffers.
const MAX_FRAME_LEN: usize = 5760 * CHANNELS;

pub struct OpusEncoder {
    inner: Encoder,
    out: Vec<u8>,
}

impl OpusEncoder {
    pub fn new(bitrate: i32) -> Result<Self, VoiceError> {
        let mut inner = Encoder::new(SAMPLE_RATE, Channels::Stereo, Application::Voip)?;
        inner.set_bitrate(Bitrate::Bits(bitrate))?;
        // Forward error correction hides the odd lost packet.
        inner.set_inband_fec(true)?;
        inner.set_packet_loss_perc(5)?;
        Ok(OpusEncoder {
            inner,
            out: vec![0; 4000],
        })
    }

    /// Encode one frame of interleaved stereo samples.
    pub fn encode(&mut self, pcm: &[f32]) -> Result<&[u8], VoiceError> {
        let len = self.inner.encode_float(pcm, &mut self.out)?;
        Ok(&self.out[..len])
    }
}

pub struct OpusDecoder {
    inner: Decoder,
    out: Vec<f32>,
}

impl OpusDecoder {
    pub fn new() -> Result<Self, VoiceError> {
        Ok(OpusDecoder {
            inner: Decoder::new(SAMPLE_RATE, Channels::Stereo)?,
            out: vec![0.0; MAX_FRAME_LEN],
        })
    }

    /// Decode one packet into interleaved stereo samples.
    pub fn decode(&mut self, packet: &[u8]) -> Result<&[f32], VoiceError> {
        let samples = self.inner.decode_float(packet, &mut self.out, false)?;
        Ok(&self.out[..samples * CHANNELS])
    }
}

/// Decides when to transmit, so we don't send background hiss.
///
/// Opens when a frame is louder than the threshold and stays open a little
/// longer after the last loud frame, so word endings aren't clipped.
pub struct VoiceGate {
    threshold_db: f32,
    hangover: u32,
    remaining: u32,
}

/// 300 ms of frames.
const HANGOVER_FRAMES: u32 = 15;

impl VoiceGate {
    pub fn new(threshold_db: f32) -> Self {
        VoiceGate {
            threshold_db,
            hangover: HANGOVER_FRAMES,
            remaining: 0,
        }
    }

    pub fn set_threshold(&mut self, threshold_db: f32) {
        self.threshold_db = threshold_db;
    }

    /// Returns whether this frame should be sent.
    pub fn process(&mut self, pcm: &[f32]) -> bool {
        if level_db(pcm) >= self.threshold_db {
            self.remaining = self.hangover;
            true
        } else if self.remaining > 0 {
            self.remaining -= 1;
            true
        } else {
            false
        }
    }

    pub fn close(&mut self) {
        self.remaining = 0;
    }
}

/// Cleans up the microphone before it's encoded: makes it proper mono, applies
/// the manual gain, then automatic gain so quiet mics come out at a normal
/// level (like Discord's own "automatic gain control"), with a soft limiter
/// so boosted speech doesn't clip.
pub struct MicProcessor {
    manual_gain: f32,
    auto_gain: bool,
    /// The automatic gain right now (linear).
    agc: f32,
    /// RNNoise, when noise suppression is on.
    denoise: Option<Box<nnnoiseless::DenoiseState<'static>>>,
    scratch: Vec<f32>,
}

/// Where automatic gain aims for speech, in dBFS RMS.
const AGC_TARGET_DB: f32 = -20.0;
const AGC_MAX: f32 = 8.0; // +18 dB
const AGC_MIN: f32 = 0.5; // -6 dB

impl MicProcessor {
    pub fn new(gain_db: f32, auto_gain: bool) -> Self {
        MicProcessor {
            manual_gain: db_to_linear(gain_db),
            auto_gain,
            agc: 1.0,
            denoise: None,
            scratch: Vec::new(),
        }
    }

    /// Remove background noise (fans, keyboards) with RNNoise.
    pub fn set_noise_suppression(&mut self, on: bool) {
        if on && self.denoise.is_none() {
            self.denoise = Some(nnnoiseless::DenoiseState::new());
        } else if !on {
            self.denoise = None;
        }
    }

    /// Step 1, before deciding whether to transmit: proper mono, minus
    /// background noise.
    pub fn clean(&mut self, frame: &mut [f32]) {
        to_mono(frame);
        let Some(denoise) = &mut self.denoise else {
            return;
        };
        // RNNoise wants mono, 480 samples at a time, scaled like 16-bit audio.
        const N: usize = nnnoiseless::DenoiseState::FRAME_SIZE;
        let mono: Vec<f32> = frame
            .iter()
            .step_by(CHANNELS)
            .map(|s| s * 32768.0)
            .collect();
        self.scratch.resize(mono.len(), 0.0);
        for (input, output) in mono
            .as_chunks::<N>()
            .0
            .iter()
            .zip(self.scratch.as_chunks_mut::<N>().0.iter_mut())
        {
            denoise.process_frame(output, input);
        }
        for (pair, &sample) in frame
            .as_chunks_mut::<CHANNELS>()
            .0
            .iter_mut()
            .zip(&self.scratch)
        {
            let value = sample / 32768.0;
            pair.fill(value);
        }
    }

    pub fn set_gain(&mut self, gain_db: f32, auto_gain: bool) {
        self.manual_gain = db_to_linear(gain_db);
        self.auto_gain = auto_gain;
    }

    /// Both steps at once (see [`Self::clean`] and [`Self::amplify`]).
    pub fn process(&mut self, frame: &mut [f32], speech: bool) {
        self.clean(frame);
        self.amplify(frame, speech);
    }

    /// Step 2, for frames being sent: manual and automatic gain. `speech`
    /// says whether this is talking (only speech moves the auto gain, so
    /// background noise isn't pumped up between words).
    pub fn amplify(&mut self, frame: &mut [f32], speech: bool) {
        let mut gain = self.manual_gain;
        if self.auto_gain {
            if speech {
                let level = level_db(frame) + 20.0 * gain.log10();
                let wanted = db_to_linear(AGC_TARGET_DB - level).clamp(AGC_MIN, AGC_MAX);
                // Turn up slowly, turn down quickly.
                let speed = if wanted > self.agc { 0.03 } else { 0.3 };
                self.agc += (wanted - self.agc) * speed;
            }
            gain *= self.agc;
        }
        for sample in frame.iter_mut() {
            *sample = soft_limit(*sample * gain);
        }
    }
}

/// Mono headsets often fill only the left channel, which sounds half as
/// loud to others. Use the channel that has audio, or average both.
fn to_mono(frame: &mut [f32]) {
    let (mut left, mut right) = (0.0f32, 0.0f32);
    for pair in frame.as_chunks::<2>().0 {
        left += pair[0] * pair[0];
        right += pair[1] * pair[1];
    }
    let pick = |pair: &[f32; 2]| {
        if right < left * 0.05 {
            pair[0]
        } else if left < right * 0.05 {
            pair[1]
        } else {
            (pair[0] + pair[1]) * 0.5
        }
    };
    for pair in frame.as_chunks_mut::<2>().0 {
        let mono = pick(pair);
        pair[0] = mono;
        pair[1] = mono;
    }
}

/// Untouched below 0.8, then rounds off smoothly instead of clipping at 1.0.
fn soft_limit(sample: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let magnitude = sample.abs();
    if magnitude <= KNEE {
        sample
    } else {
        let over = (magnitude - KNEE) / (1.0 - KNEE);
        sample.signum() * (KNEE + (1.0 - KNEE) * over.tanh())
    }
}

fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Push-to-talk keeps sending for a moment after the key is let go, so the
/// last word isn't cut off.
#[derive(Default)]
pub struct Hold {
    remaining: u32,
}

impl Hold {
    /// 200 ms of frames.
    const FRAMES: u32 = 10;

    pub fn process(&mut self, held: bool) -> bool {
        if held {
            self.remaining = Self::FRAMES;
            true
        } else if self.remaining > 0 {
            self.remaining -= 1;
            true
        } else {
            false
        }
    }
}

/// Loudness of a frame in dBFS (0 is the loudest possible).
pub fn level_db(pcm: &[f32]) -> f32 {
    if pcm.is_empty() {
        return f32::NEG_INFINITY;
    }
    let mean_square = pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32;
    10.0 * mean_square.max(1e-12).log10()
}

impl From<opus::Error> for VoiceError {
    fn from(error: opus::Error) -> Self {
        VoiceError::Codec(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32) -> Vec<f32> {
        (0..FRAME_LEN)
            .map(|i| amplitude * ((i / 2) as f32 * 0.05).sin())
            .collect()
    }

    #[test]
    fn encode_then_decode_keeps_the_frame_size() {
        let mut encoder = OpusEncoder::new(64_000).unwrap();
        let mut decoder = OpusDecoder::new().unwrap();
        let packet = encoder.encode(&tone(0.5)).unwrap().to_vec();
        assert!(!packet.is_empty());
        assert_eq!(decoder.decode(&packet).unwrap().len(), FRAME_LEN);
    }

    #[test]
    fn gate_opens_on_speech_and_holds_briefly() {
        let mut gate = VoiceGate::new(-40.0);
        assert!(!gate.process(&tone(0.0)));
        assert!(gate.process(&tone(0.5)));
        for _ in 0..HANGOVER_FRAMES {
            assert!(gate.process(&tone(0.0)));
        }
        assert!(!gate.process(&tone(0.0)));
    }

    #[test]
    fn auto_gain_brings_quiet_speech_up_without_clipping() {
        let mut mic = MicProcessor::new(0.0, true);
        let mut frame = tone(0.02); // about -37 dBFS
        for _ in 0..200 {
            frame = tone(0.02);
            mic.process(&mut frame, true);
        }
        let level = level_db(&frame);
        assert!(level > -25.0 && level < -15.0, "level {level}");

        let mut loud = tone(1.0);
        MicProcessor::new(12.0, false).process(&mut loud, true);
        assert!(loud.iter().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn noise_suppression_runs_on_whole_frames() {
        // RNNoise is trained on real recordings, so synthetic noise says
        // little about quality; this checks the plumbing.
        let mut mic = MicProcessor::new(0.0, false);
        mic.set_noise_suppression(true);
        for _ in 0..20 {
            let mut frame = tone(0.3);
            mic.clean(&mut frame);
            assert_eq!(frame.len(), FRAME_LEN);
            assert!(frame.iter().all(|s| s.is_finite() && s.abs() <= 1.5));
            assert!(frame.as_chunks::<2>().0.iter().all(|p| p[0] == p[1]));
        }
        mic.set_noise_suppression(false);
        let mut frame = tone(0.3);
        let before = frame.clone();
        mic.clean(&mut frame);
        assert_eq!(frame, before);
    }

    #[test]
    fn left_only_mic_becomes_both_channels() {
        let mut frame: Vec<f32> = (0..FRAME_LEN)
            .map(|i| if i % 2 == 0 { 0.3 } else { 0.0 })
            .collect();
        to_mono(&mut frame);
        assert!(frame.iter().all(|&s| s == 0.3));
    }

    #[test]
    fn levels() {
        assert!(level_db(&tone(0.0)) < -100.0);
        assert!(level_db(&tone(1.0)) > -4.0);
    }
}
