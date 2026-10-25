//! Check your microphone and speakers without Discord: records, runs the
//! audio through Opus like a call would, and plays it back to you.
//!
//! ```text
//! cargo run -p plume-voice --example mic_test -- 10
//! ```
//!
//! Use headphones, or you'll hear feedback.

use std::time::{Duration, Instant};

use plume_voice::audio_io;
use plume_voice::opus::{MicProcessor, OpusDecoder, OpusEncoder, level_db};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let (frames_tx, mut frames) = tokio::sync::mpsc::channel(8);
    let audio = audio_io::start(frames_tx, None, None).await;
    println!(
        "microphone: {}",
        if audio.has_input { "ok" } else { "not found" }
    );
    println!(
        "speakers:   {}",
        if audio.has_output { "ok" } else { "not found" }
    );
    if !audio.has_input {
        return;
    }

    let mut encoder = OpusEncoder::new(64_000).expect("opus encoder");
    let mut decoder = OpusDecoder::new().expect("opus decoder");
    // Same processing as a call (automatic gain on), so you hear what others hear.
    let mut mic = MicProcessor::new(0.0, true);
    mic.set_noise_suppression(true);
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut count = 0u32;
    let mut loudest = f32::NEG_INFINITY;
    while let Ok(Some(mut frame)) = tokio::time::timeout_at(end.into(), frames.recv()).await {
        let level = level_db(&frame);
        loudest = loudest.max(level);
        mic.process(&mut frame, level >= -50.0);
        let packet = encoder.encode(&frame).expect("encode").to_vec();
        audio
            .mixer
            .push(0, decoder.decode(&packet).expect("decode"));
        count += 1;
        if count.is_multiple_of(10) {
            // A level meter (loudest frame of the last 0.2 s): -60 dB and below is silence.
            let db = std::mem::replace(&mut loudest, f32::NEG_INFINITY).max(-60.0);
            let bars = ((db + 60.0) / 2.0) as usize;
            println!("{db:6.1} dB {}", "#".repeat(bars));
        }
    }
    println!("{count} frames ({} s of audio)", count / 50);
}
