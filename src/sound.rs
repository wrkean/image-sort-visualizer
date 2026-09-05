//! Procedural sorting sounds.
//!
//! Every sorting event (comparison, swap, set) maps to one or more short
//! decaying sine tones whose pitch is derived from the values being acted on.
//! All tones produced within a single animation frame are summed into one
//! short buffer and played once, so the audio stays locked to the animation
//! even when a frame carries many events (dense frames naturally collapse into
//! a glissando/chord instead of a long backlog).
//!
//! Audio output is best-effort: if no output device is available the `Sound`
//! type degrades to a silent no-op rather than taking the program down.

use std::num::NonZero;

use rodio::buffer::SamplesBuffer;
use rodio::mixer::Mixer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};

/// Sample rate used for the synthesized audio (mono).
const SAMPLE_RATE: u32 = 48_000;

/// Lowest pitch in the scale (a fairly low, non-muddy A).
const BASE_FREQ: f32 = 220.0;

/// Each sort key maps to a pitch spanning this many octaves upward (A3 → A6).
const OCTAVES: f32 = 3.0;

/// Length of a mixed per-frame buffer, in seconds.
const FRAME_SECS: f32 = 0.06;

/// Exponential-decay time constant, seconds. A tone falls to ~8% of its
/// initial amplitude by the buffer's end, giving a short percussive blip.
const TAU: f32 = FRAME_SECS / 2.5;

/// Peak amplitude for a single tone, before per-event volume scaling.
const AMP: f32 = 0.30;

/// Cap on how many (unplayed) frame buffers may be queued in one pane's
/// player. When the output falls behind (e.g. a dense burst of event-heavy
/// frames) new frames are dropped rather than accumulating latency, keeping
/// the sound tied to the animation instead of lagging far behind it.
const MAX_QUEUED_FRAMES: usize = 5;

/// If a single frame produced more tones than this, the tones are subsampled
/// evenly (first/last kept) so dense frames become a musical glissando rather
/// than an unintelligible noise cluster.
const MAX_TONES_PER_FRAME: usize = 16;

/// One tone to render, as a normalized sort key `0..=1` and loudness `0..=1`.
pub type Tone = (f32, f32);

/// Owns the open output stream. `Sound`s created from it share the same
/// physical output device.
pub struct Output {
    /// Held so the device stream isn't dropped while sounds play through it.
    _device: MixerDeviceSink,
    /// Shared mixer on that device; Sounds connect their own players to it.
    mixer: Mixer,
}

/// A tone generator for one sort pane. Each `Sound` keeps its own sequential
/// `Player` on the shared mixer, so tones from concurrent sort panes play in
/// parallel and mix naturally.
pub struct Sound {
    player: Player,
    /// Whether tones are currently silenced.
    muted: bool,
}

impl Output {
    /// Open the default output device. Returns `None` when no audio device is
    /// available (e.g. headless), so callers can fall back to silence.
    pub fn open() -> Option<Output> {
        let device = DeviceSinkBuilder::open_default_sink().ok()?;
        let mixer = device.mixer().clone();
        Some(Output {
            _device: device,
            mixer,
        })
    }

    /// Create a per-pane generator sharing this device output.
    pub fn sound(&self) -> Sound {
        let player = Player::connect_new(&self.mixer);
        Sound { player, muted: false }
    }
}

impl Sound {
    /// Set/clear mute and return the new state.
    pub fn set_muted(&mut self, muted: bool) -> bool {
        self.muted = muted;
        self.muted
    }

    /// Whether this generator is currently silenced.
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Render one frame's worth of tones. Mixes `tones` (normalized sort key,
    /// loudness) into a single decaying-sine buffer and queues it on the
    /// player. Quiet frames produce one or two distinct notes; busy frames
    /// produce a pitched glissando chord.
    pub fn frame(&self, tones: &[Tone]) {
        if self.muted || tones.is_empty() {
            return;
        }
        // If the player is already backed up, drop this frame rather than
        // letting the sound run further ahead of the animation.
        if self.player.len() >= MAX_QUEUED_FRAMES {
            return;
        }

        // Evenly subsample dense frames, keeping the first and last tones so
        // the pitch motion across the frame is preserved. Integer arithmetic
        // guarantees indices stay within bounds.
        let sampled: Vec<Tone> = if tones.len() > MAX_TONES_PER_FRAME {
            let count = tones.len();
            (0..MAX_TONES_PER_FRAME)
                .map(|i| tones[(i * (count - 1)) / (MAX_TONES_PER_FRAME - 1)])
                .collect()
        } else {
            tones.to_vec()
        };
        let count = sampled.len();

        // Loudness scales down gently with the number of simultaneous tones so
        // a full glissando chord isn't six times as loud as a single note.
        let volume_scale = 1.0 / (count as f32).sqrt();

        let n = (FRAME_SECS * SAMPLE_RATE as f32) as usize;
        let mut samples = vec![0.0f32; n];
        // Per-sample exponential decay: the envelope falls to exp(-FRAME/TAU)
        // ≈ 8% by the buffer's end.
        let decay = (-1.0 / (SAMPLE_RATE as f32 * TAU)).exp();

        for &(value, volume) in &sampled {
            let freq = BASE_FREQ * (1.0 + value.clamp(0.0, 1.0)).powf(OCTAVES);
            let gain = AMP * volume.clamp(0.0, 1.0) * volume_scale;
            let mut phase = 0.0f32;
            let phase_inc = 2.0 * std::f32::consts::PI * freq / SAMPLE_RATE as f32;
            let mut sample = gain;
            for s in samples.iter_mut() {
                *s += phase.sin() * sample;
                phase += phase_inc;
                if phase > 2.0 * std::f32::consts::PI {
                    phase -= 2.0 * std::f32::consts::PI;
                }
                sample *= decay;
            }
        }

        // Soft-clip so overlapping partials can't exceed the device's range.
        for s in samples.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }

        let src = SamplesBuffer::new(
            NonZero::new(1).unwrap(),
            NonZero::new(SAMPLE_RATE).unwrap(),
            samples,
        );
        self.player.append(src);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_mapping_is_monotonic_and_sane() {
        let f_lo = BASE_FREQ * (1.0f32 + 0.0).powf(OCTAVES);
        let f_hi = BASE_FREQ * (1.0f32 + 1.0).powf(OCTAVES);
        assert!((200.0..400.0).contains(&f_lo), "low pitch {f_lo} out of range");
        assert!((1700.0..=1800.0).contains(&f_hi), "high pitch {f_hi} out of range");
        // Higher values always map to strictly higher pitches.
        for i in 0..99 {
            let a = i as f32 / 99.0;
            let b = (i as f32 + 1.0) / 99.0;
            let fa = BASE_FREQ * (1.0 + a).powf(OCTAVES);
            let fb = BASE_FREQ * (1.0 + b).powf(OCTAVES);
            assert!(fa < fb, "pitch not monotonic at {i}: {fa} !< {fb}");
        }
    }

    #[test]
    fn frame_sample_count_matches_duration() {
        let n = (FRAME_SECS * SAMPLE_RATE as f32) as usize;
        // 0.06 s * 48000 Hz = 2880 samples.
        assert_eq!(n, 2880);
    }

    #[test]
    fn envelope_decays_smoothly() {
        let n = (FRAME_SECS * SAMPLE_RATE as f32) as usize;
        let decay = (-1.0 / (SAMPLE_RATE as f32 * TAU)).exp();
        assert!(decay < 1.0, "decay must shrink each sample");
        let mut env = AMP;
        let mut peak = 0.0f32;
        for _ in 0..n {
            peak = peak.max(env.abs());
            env *= decay;
        }
        assert!(peak <= AMP);
        // By the end of the frame the envelope has fallen to ~8% of its start.
        assert!((env / AMP).abs() < 0.15, "tail too loud: {}", env / AMP);
        // Decay is smooth — no adjacent sample jumps by more than 1/1000.
        let step = 1.0 - decay;
        assert!(step < 0.001, "decay per sample too harsh: {step}");
    }

    #[test]
    fn subsampling_preserves_endpoints() {
        for len in [17usize, 100, 200, 1000] {
            let tones: Vec<Tone> = (0..len)
                .map(|i| (i as f32 / (len - 1) as f32, 0.5))
                .collect();
            // Manual mirror of the production subsample logic.
            let sampled: Vec<Tone> = (0..MAX_TONES_PER_FRAME)
                .map(|i| tones[(i * (len - 1)) / (MAX_TONES_PER_FRAME - 1)])
                .collect();
            assert_eq!(sampled.len(), MAX_TONES_PER_FRAME);
            assert_eq!(sampled[0], tones[0], "first tone must be kept");
            assert_eq!(
                sampled[MAX_TONES_PER_FRAME - 1],
                tones[len - 1],
                "last tone must be kept (len={len})"
            );
            // Subsample is monotonically non-decreasing in pitch and in-bounds.
            assert!(sampled.windows(2).all(|w| w[0].0 < w[1].0));
        }
    }
}