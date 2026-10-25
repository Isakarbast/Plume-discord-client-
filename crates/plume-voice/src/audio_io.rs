//! Microphone and speakers, through `cpal`.
//!
//! Everything inside the voice code is 48 kHz stereo `f32`, which is what
//! Opus wants. Devices that run at other rates or channel counts are
//! converted here.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, FromSample, SampleFormat, SizedSample, StreamConfig};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::opus::{CHANNELS, FRAME_LEN, SAMPLE_RATE};

/// Wait for this much audio before playing someone, to ride out jitter.
const PREBUFFER: usize = 3 * FRAME_LEN;
/// Drop audio beyond this, so latency can't grow without bound.
const MAX_BUFFER: usize = 12 * FRAME_LEN;

/// Running audio devices. Dropping this closes them.
pub struct AudioIo {
    pub mixer: Arc<Mixer>,
    pub has_input: bool,
    pub has_output: bool,
    _stop: std::sync::mpsc::Sender<()>,
}

/// Open the default microphone and speakers. Each captured 20 ms frame is
/// sent on `frames`. Missing devices aren't fatal: you can still listen
/// without a mic.
pub async fn start(
    frames: mpsc::Sender<Vec<f32>>,
    input: Option<String>,
    output: Option<String>,
) -> AudioIo {
    let mixer = Arc::new(Mixer::default());
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    let (ready, opened) = oneshot::channel();

    // Some backends' streams can't move between threads, so they live on
    // their own thread until we're told to stop.
    let thread_mixer = mixer.clone();
    std::thread::Builder::new()
        .name("plume-audio".into())
        .spawn(move || {
            let host = cpal::default_host();
            let named = |devices: Option<Vec<cpal::Device>>, name: &Option<String>| {
                let name = name.as_deref()?;
                devices?
                    .into_iter()
                    .find(|d| device_name(d).as_deref() == Some(name))
            };
            let input_device = named(host.input_devices().ok().map(Iterator::collect), &input)
                .or_else(|| host.default_input_device());
            let output_device = named(host.output_devices().ok().map(Iterator::collect), &output)
                .or_else(|| host.default_output_device());
            let input = input_device
                .ok_or_else(|| "no microphone found".to_owned())
                .and_then(|device| open_input(&device, frames));
            let output = output_device
                .ok_or_else(|| "no speakers found".to_owned())
                .and_then(|device| open_output(&device, thread_mixer));
            for (what, result) in [("microphone", &input), ("speakers", &output)] {
                if let Err(error) = result {
                    warn!("{what}: {error}");
                }
            }
            let _ = ready.send((input.is_ok(), output.is_ok()));
            // Blocks until `AudioIo` is dropped, keeping the streams alive.
            let _ = stopped.recv();
            debug!("audio devices closed");
        })
        .expect("failed to start the audio thread");

    let (has_input, has_output) = opened.await.unwrap_or((false, false));
    AudioIo {
        mixer,
        has_input,
        has_output,
        _stop: stop,
    }
}

fn device_name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_owned())
}

/// Microphones to choose from, by name.
pub fn input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.filter_map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

/// Speakers and headphones to choose from, by name.
pub fn output_devices() -> Vec<String> {
    cpal::default_host()
        .output_devices()
        .map(|devices| devices.filter_map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

fn open_input(
    device: &cpal::Device,
    frames: mpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream, String> {
    let mut last_error = String::new();
    for (config, format) in candidate_configs(device.default_input_config()) {
        let result = match format {
            SampleFormat::F32 => build_input::<f32>(device, config, frames.clone()),
            SampleFormat::I16 => build_input::<i16>(device, config, frames.clone()),
            SampleFormat::I32 => build_input::<i32>(device, config, frames.clone()),
            SampleFormat::U16 => build_input::<u16>(device, config, frames.clone()),
            other => Err(format!("unsupported sample format {other:?}")),
        };
        match result {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn open_output(device: &cpal::Device, mixer: Arc<Mixer>) -> Result<cpal::Stream, String> {
    let mut last_error = String::new();
    for (config, format) in candidate_configs(device.default_output_config()) {
        let result = match format {
            SampleFormat::F32 => build_output::<f32>(device, config, mixer.clone()),
            SampleFormat::I16 => build_output::<i16>(device, config, mixer.clone()),
            SampleFormat::I32 => build_output::<i32>(device, config, mixer.clone()),
            SampleFormat::U16 => build_output::<u16>(device, config, mixer.clone()),
            other => Err(format!("unsupported sample format {other:?}")),
        };
        match result {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

/// 48 kHz stereo float first (no conversion needed), then whatever the
/// device says it prefers.
fn candidate_configs(
    default: Result<cpal::SupportedStreamConfig, cpal::Error>,
) -> Vec<(StreamConfig, SampleFormat)> {
    let mut configs = vec![(
        StreamConfig {
            channels: CHANNELS as u16,
            sample_rate: SAMPLE_RATE,
            buffer_size: BufferSize::Default,
        },
        SampleFormat::F32,
    )];
    match default {
        Ok(default) => configs.push((default.config(), default.sample_format())),
        Err(error) => debug!("no default audio config: {error}"),
    }
    configs
}

fn build_input<T>(
    device: &cpal::Device,
    config: StreamConfig,
    frames: mpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let mut capture = Capture::new(config.sample_rate, frames);
    let stream = device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                for frame in data.chunks_exact(channels) {
                    let left = frame[0].to_sample::<f32>();
                    let right = frame.get(1).map_or(left, |&s| s.to_sample::<f32>());
                    capture.push([left, right]);
                }
            },
            |error| warn!("microphone error: {error}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build_output<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mixer: Arc<Mixer>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    let mut playback = Playback::new(config.sample_rate);
    let stream = device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], _| {
                let mut inner = mixer.lock();
                for frame in data.chunks_exact_mut(channels) {
                    let [left, right] = playback.next(&mut inner);
                    if channels == 1 {
                        frame[0] = T::from_sample((left + right) * 0.5);
                    } else {
                        frame[0] = T::from_sample(left);
                        frame[1] = T::from_sample(right);
                        for extra in &mut frame[2..] {
                            *extra = T::from_sample(0.0);
                        }
                    }
                }
            },
            |error| warn!("speaker error: {error}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Turns microphone samples into 48 kHz frames.
struct Capture {
    resampler: Resampler,
    frame: Vec<f32>,
    frames: mpsc::Sender<Vec<f32>>,
}

impl Capture {
    fn new(rate: u32, frames: mpsc::Sender<Vec<f32>>) -> Self {
        Capture {
            resampler: Resampler::new(rate, SAMPLE_RATE),
            frame: Vec::with_capacity(FRAME_LEN),
            frames,
        }
    }

    fn push(&mut self, sample: [f32; 2]) {
        let Capture {
            resampler,
            frame,
            frames,
        } = self;
        resampler.push(sample, |out| {
            frame.extend_from_slice(&out);
            if frame.len() == FRAME_LEN {
                // If the voice task is behind, dropping a frame beats
                // building up delay.
                let _ = frames.try_send(std::mem::replace(frame, Vec::with_capacity(FRAME_LEN)));
            }
        });
    }
}

/// Linear resampling between two rates. Plenty for speech.
struct Resampler {
    /// Input frames per output frame.
    step: f64,
    pos: f64,
    last: [f32; 2],
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Resampler {
            step: f64::from(from) / f64::from(to),
            pos: 0.0,
            last: [0.0; 2],
        }
    }

    /// Feed one input frame; calls `emit` for each output frame it yields.
    fn push(&mut self, sample: [f32; 2], mut emit: impl FnMut([f32; 2])) {
        while self.pos < 1.0 {
            emit(lerp(self.last, sample, self.pos as f32));
            self.pos += self.step;
        }
        self.pos -= 1.0;
        self.last = sample;
    }
}

/// Pulls 48 kHz frames from the mixer at the speaker's rate.
struct Playback {
    step: f64,
    pos: f64,
    prev: [f32; 2],
    next: [f32; 2],
}

impl Playback {
    fn new(rate: u32) -> Self {
        Playback {
            step: f64::from(SAMPLE_RATE) / f64::from(rate),
            pos: 1.0,
            prev: [0.0; 2],
            next: [0.0; 2],
        }
    }

    fn next(&mut self, mixer: &mut MixerInner) -> [f32; 2] {
        while self.pos >= 1.0 {
            self.prev = self.next;
            self.next = mixer.next_frame();
            self.pos -= 1.0;
        }
        let out = lerp(self.prev, self.next, self.pos as f32);
        self.pos += self.step;
        out
    }
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Mixes everyone's decoded audio for the speakers.
#[derive(Default)]
pub struct Mixer {
    inner: Mutex<MixerInner>,
    deafened: AtomicBool,
}

#[derive(Default)]
pub struct MixerInner {
    /// Per speaker (by SSRC): queued interleaved stereo samples.
    sources: HashMap<u32, Source>,
}

struct Source {
    samples: VecDeque<f32>,
    playing: bool,
    gain: f32,
}

impl Default for Source {
    fn default() -> Self {
        Source {
            samples: VecDeque::new(),
            playing: false,
            gain: 1.0,
        }
    }
}

impl Mixer {
    fn lock(&self) -> MutexGuard<'_, MixerInner> {
        // A panic while holding the lock would only lose some audio.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push(&self, ssrc: u32, pcm: &[f32]) {
        if self.deafened.load(Ordering::Relaxed) {
            return;
        }
        let mut inner = self.lock();
        let source = inner.sources.entry(ssrc).or_default();
        source.samples.extend(pcm);
        if source.samples.len() > MAX_BUFFER {
            let excess = source.samples.len() - MAX_BUFFER;
            source.samples.drain(..excess);
        }
    }

    /// Volume for one person (1.0 = as sent, up to 2.0).
    pub fn set_gain(&self, ssrc: u32, gain: f32) {
        self.lock().sources.entry(ssrc).or_default().gain = gain.clamp(0.0, 2.0);
    }

    pub fn remove(&self, ssrc: u32) {
        self.lock().sources.remove(&ssrc);
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.deafened.store(deafened, Ordering::Relaxed);
        if deafened {
            for source in self.lock().sources.values_mut() {
                source.samples.clear();
                source.playing = false;
            }
        }
    }
}

impl MixerInner {
    fn next_frame(&mut self) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for source in self.sources.values_mut() {
            if !source.playing {
                if source.samples.len() < PREBUFFER {
                    continue;
                }
                source.playing = true;
            }
            match (source.samples.pop_front(), source.samples.pop_front()) {
                (Some(left), Some(right)) => {
                    out[0] += left * source.gain;
                    out[1] += right * source.gain;
                }
                // Ran dry: buffer up again before resuming.
                _ => source.playing = false,
            }
        }
        out.map(|s| s.clamp(-1.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_produces_the_right_amount() {
        for (from, to) in [
            (48_000, 48_000),
            (44_100, 48_000),
            (96_000, 48_000),
            (16_000, 48_000),
        ] {
            let mut resampler = Resampler::new(from, to);
            let mut produced = 0usize;
            for _ in 0..from {
                resampler.push([0.1, 0.1], |_| produced += 1);
            }
            assert!(
                produced.abs_diff(to as usize) <= 1,
                "{from}->{to}: {produced}"
            );
        }
    }

    #[test]
    fn mixer_prebuffers_then_sums_sources() {
        let mixer = Mixer::default();
        mixer.push(1, &[0.25; FRAME_LEN]);
        // Not enough buffered yet: silence.
        assert_eq!(mixer.lock().next_frame(), [0.0, 0.0]);

        mixer.push(1, &[0.25; 2 * FRAME_LEN]);
        mixer.push(2, &[0.5; 3 * FRAME_LEN]);
        assert_eq!(mixer.lock().next_frame(), [0.75, 0.75]);
    }

    #[test]
    fn deafened_mixer_stays_silent() {
        let mixer = Mixer::default();
        mixer.set_deafened(true);
        mixer.push(1, &[0.5; 4 * FRAME_LEN]);
        assert_eq!(mixer.lock().next_frame(), [0.0, 0.0]);
    }

    #[test]
    fn playback_at_another_rate_pulls_proportionally() {
        let mut inner = MixerInner::default();
        inner.sources.insert(
            1,
            Source {
                samples: vec![0.1; 2 * 48_000].into(),
                playing: true,
                gain: 1.0,
            },
        );
        let mut playback = Playback::new(24_000);
        for _ in 0..24_000 {
            playback.next(&mut inner);
        }
        let left = inner.sources[&1].samples.len() / 2;
        assert!(left <= 2, "{left} frames left");
    }
}
