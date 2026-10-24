// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The computer's microphone and speaker, through cpal (Core Audio on
//! macOS, ALSA on Linux).
//!
//! Devices run at their own rate and channel count. Capture mixes down to
//! mono, resamples to 48 kHz and cuts 20 ms frames; playback resamples from
//! 48 kHz and copies the mono signal to every channel. The audio callbacks
//! never block: capture hands frames over a bounded channel (a frame is
//! dropped when the engine falls behind), playback reads a bounded queue
//! (silence when it runs dry, the oldest audio dropped when it overfills).
//!
//! No echo cancellation in v1: `webrtc-audio-processing` builds WebRTC's
//! audio processing with meson, which neither the macOS nor the Linux build
//! has; calls are headset-only until it does (or until macOS's voice
//! processing I/O unit is used). See `docs/contract.md`.
//!
//! On macOS the first capture asks for microphone access (the bundle's
//! `NSMicrophoneUsageDescription`); while access is denied Core Audio
//! delivers silence, which the engine reports as a blocked microphone.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};

use crate::codec::{FRAME_SAMPLES, Frame, SAMPLE_RATE};
use crate::engine::{Capture, Playback};

/// Captured frames queued for the engine (100 ms).
const CAPTURE_QUEUE: usize = 5;
/// Most audio queued for the speaker (200 ms at 48 kHz).
const PLAYBACK_QUEUE_MS: usize = 200;

/// An audio device the user can choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Stable id (what a choice is saved as).
    pub id: String,
    /// Name shown.
    pub name: String,
    /// The system default for its direction.
    pub is_default: bool,
}

/// Which devices to use; `None` is the system default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceChoice {
    /// Microphone id.
    pub input: Option<String>,
    /// Speaker id.
    pub output: Option<String>,
}

/// Why audio devices could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// No microphone (or not the chosen one).
    #[error("no microphone available")]
    NoInput,
    /// No speaker (or not the chosen one).
    #[error("no speaker available")]
    NoOutput,
    /// The device refused the stream.
    #[error("audio device: {0}")]
    Stream(String),
}

fn device_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

fn device_name(device: &cpal::Device) -> String {
    device.description().map(|d| d.name().to_owned()).unwrap_or_else(|_| "?".to_owned())
}

fn list(devices: impl Iterator<Item = cpal::Device>, default: Option<String>) -> Vec<DeviceInfo> {
    devices
        .filter_map(|device| {
            let id = device_id(&device)?;
            Some(DeviceInfo {
                is_default: default.as_ref() == Some(&id),
                name: device_name(&device),
                id,
            })
        })
        .collect()
}

/// The microphones.
pub fn inputs() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let default = host.default_input_device().as_ref().and_then(device_id);
    host.input_devices().map(|devices| list(devices, default)).unwrap_or_default()
}

/// The speakers.
pub fn outputs() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let default = host.default_output_device().as_ref().and_then(device_id);
    host.output_devices().map(|devices| list(devices, default)).unwrap_or_default()
}

/// Open device streams; dropping this closes them.
pub struct Streams {
    _input: Stream,
    _output: Stream,
}

impl std::fmt::Debug for Streams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Streams")
    }
}

/// Opens the chosen microphone and speaker.
///
/// # Errors
///
/// A device is missing or refuses its stream.
pub fn open(
    choice: &DeviceChoice,
) -> Result<(DeviceCapture, DevicePlayback, Streams), DeviceError> {
    let host = cpal::default_host();
    let pick = |id: &Option<String>, mut all: Box<dyn Iterator<Item = cpal::Device>>| {
        id.as_ref().and_then(|id| all.find(|d| device_id(d).as_ref() == Some(id)))
    };
    let input = match &choice.input {
        Some(_) => pick(&choice.input, Box::new(host.input_devices().into_iter().flatten())),
        None => host.default_input_device(),
    }
    .ok_or(DeviceError::NoInput)?;
    let output = match &choice.output {
        Some(_) => pick(&choice.output, Box::new(host.output_devices().into_iter().flatten())),
        None => host.default_output_device(),
    }
    .ok_or(DeviceError::NoOutput)?;

    let (frames_tx, frames_rx) = sync_channel(CAPTURE_QUEUE);
    let input_stream = open_input(&input, frames_tx)?;
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let (output_stream, output_rate) = open_output(&output, queue.clone())?;
    input_stream.play().map_err(|e| DeviceError::Stream(e.to_string()))?;
    output_stream.play().map_err(|e| DeviceError::Stream(e.to_string()))?;
    Ok((
        DeviceCapture { frames: frames_rx },
        DevicePlayback {
            queue,
            resampler: Resampler::new(SAMPLE_RATE, output_rate),
            limit: output_rate as usize * PLAYBACK_QUEUE_MS / 1000,
        },
        Streams { _input: input_stream, _output: output_stream },
    ))
}

fn open_input(device: &cpal::Device, frames: SyncSender<Frame>) -> Result<Stream, DeviceError> {
    let supported =
        device.default_input_config().map_err(|e| DeviceError::Stream(e.to_string()))?;
    let config: StreamConfig = supported.config();
    let cutter = Cutter::new(config.sample_rate, usize::from(config.channels), frames);
    macro_rules! build {
        ($t:ty) => {{
            let mut cutter = cutter;
            device.build_input_stream(
                config,
                move |data: &[$t], _: &cpal::InputCallbackInfo| cutter.push(data),
                |_| {},
                None,
            )
        }};
    }
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build!(f32),
        SampleFormat::I16 => build!(i16),
        SampleFormat::I32 => build!(i32),
        SampleFormat::U16 => build!(u16),
        other => return Err(DeviceError::Stream(format!("unsupported sample format {other}"))),
    };
    stream.map_err(|e| DeviceError::Stream(e.to_string()))
}

fn open_output(
    device: &cpal::Device,
    queue: Arc<Mutex<VecDeque<f32>>>,
) -> Result<(Stream, u32), DeviceError> {
    let supported =
        device.default_output_config().map_err(|e| DeviceError::Stream(e.to_string()))?;
    let config: StreamConfig = supported.config();
    let channels = usize::from(config.channels).max(1);
    macro_rules! build {
        ($t:ty) => {{
            let queue = queue.clone();
            device.build_output_stream(
                config,
                move |data: &mut [$t], _: &cpal::OutputCallbackInfo| fill(data, channels, &queue),
                |_| {},
                None,
            )
        }};
    }
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build!(f32),
        SampleFormat::I16 => build!(i16),
        SampleFormat::I32 => build!(i32),
        SampleFormat::U16 => build!(u16),
        other => return Err(DeviceError::Stream(format!("unsupported sample format {other}"))),
    };
    Ok((stream.map_err(|e| DeviceError::Stream(e.to_string()))?, config.sample_rate))
}

fn fill<T: SizedSample + FromSample<f32>>(
    data: &mut [T],
    channels: usize,
    queue: &Mutex<VecDeque<f32>>,
) {
    let Ok(mut queue) = queue.try_lock() else {
        // The engine holds the queue this instant: play silence once.
        data.fill(T::EQUILIBRIUM);
        return;
    };
    for frame in data.chunks_mut(channels) {
        let sample = queue.pop_front().unwrap_or(0.0);
        frame.fill(T::from_sample(sample));
    }
}

/// Ring tone level (of full scale): audible, not startling.
const RING_LEVEL: f32 = 0.12;
/// Ring cadence: two seconds of tone, four of silence.
const RING_ON_MS: u64 = 2_000;
const RING_PERIOD_MS: u64 = 6_000;

/// The ring tone of an incoming call on the speaker, until dropped: 440 Hz
/// plus 480 Hz, two seconds on and four off.
pub struct Ringer {
    _stream: Stream,
}

impl std::fmt::Debug for Ringer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ringer")
    }
}

impl Ringer {
    /// Starts ringing on the chosen speaker (`None`: the default one).
    ///
    /// # Errors
    ///
    /// No speaker, or it refuses the stream.
    pub fn start(output: Option<&str>) -> Result<Self, DeviceError> {
        let host = cpal::default_host();
        let device = match output {
            Some(id) => host
                .output_devices()
                .into_iter()
                .flatten()
                .find(|d| device_id(d).as_deref() == Some(id)),
            None => host.default_output_device(),
        }
        .ok_or(DeviceError::NoOutput)?;
        let supported =
            device.default_output_config().map_err(|e| DeviceError::Stream(e.to_string()))?;
        let config: StreamConfig = supported.config();
        let channels = usize::from(config.channels).max(1);
        let mut tone = RingTone::new(config.sample_rate);
        macro_rules! build {
            ($t:ty) => {
                device.build_output_stream(
                    config,
                    move |data: &mut [$t], _: &cpal::OutputCallbackInfo| {
                        for frame in data.chunks_mut(channels) {
                            frame.fill(<$t>::from_sample(tone.next_sample()));
                        }
                    },
                    |_| {},
                    None,
                )
            };
        }
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build!(f32),
            SampleFormat::I16 => build!(i16),
            SampleFormat::I32 => build!(i32),
            SampleFormat::U16 => build!(u16),
            other => return Err(DeviceError::Stream(format!("unsupported sample format {other}"))),
        }
        .map_err(|e| DeviceError::Stream(e.to_string()))?;
        stream.play().map_err(|e| DeviceError::Stream(e.to_string()))?;
        Ok(Self { _stream: stream })
    }
}

/// The ring tone's samples at one device rate.
#[derive(Debug, Clone)]
pub struct RingTone {
    rate: u64,
    position: u64,
}

impl RingTone {
    /// The tone at `rate` Hz.
    pub fn new(rate: u32) -> Self {
        Self { rate: u64::from(rate.max(1)), position: 0 }
    }

    /// The next sample.
    pub fn next_sample(&mut self) -> f32 {
        let at = self.position;
        self.position = (self.position + 1) % (self.rate * RING_PERIOD_MS / 1000);
        if at * 1000 / self.rate >= RING_ON_MS {
            return 0.0;
        }
        let t = at as f32 / self.rate as f32;
        let tau = 2.0 * std::f32::consts::PI;
        RING_LEVEL * 0.5 * ((tau * 440.0 * t).sin() + (tau * 480.0 * t).sin())
    }
}

/// A streaming linear resampler of a mono signal.
#[derive(Debug, Clone)]
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Position of the next output sample, relative to `last`.
    position: f64,
    last: f32,
}

impl Resampler {
    /// From `from` Hz to `to` Hz.
    pub fn new(from: u32, to: u32) -> Self {
        Self { step: f64::from(from) / f64::from(to.max(1)), position: 0.0, last: 0.0 }
    }

    /// Resamples `input`, appending to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &sample in input {
            // Output samples between `last` (at 0) and `sample` (at 1).
            while self.position < 1.0 {
                let t = self.position as f32;
                out.push(self.last + (sample - self.last) * t);
                self.position += self.step;
            }
            self.position -= 1.0;
            self.last = sample;
        }
    }
}

/// Cuts device audio into 48 kHz mono 20 ms frames.
struct Cutter {
    channels: usize,
    resampler: Resampler,
    mono: Vec<f32>,
    resampled: Vec<f32>,
    pending: Vec<i16>,
    frames: SyncSender<Frame>,
}

impl Cutter {
    fn new(rate: u32, channels: usize, frames: SyncSender<Frame>) -> Self {
        Self {
            channels: channels.max(1),
            resampler: Resampler::new(rate, SAMPLE_RATE),
            mono: Vec::new(),
            resampled: Vec::new(),
            pending: Vec::with_capacity(FRAME_SAMPLES * 2),
            frames,
        }
    }

    fn push<T: Sample>(&mut self, data: &[T])
    where
        f32: FromSample<T>,
    {
        self.mono.clear();
        let scale = 1.0 / self.channels as f32;
        self.mono.extend(
            data.chunks(self.channels)
                .map(|frame| frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() * scale),
        );
        self.resampled.clear();
        self.resampler.process(&self.mono, &mut self.resampled);
        self.pending.extend(self.resampled.iter().map(|s| i16::from_sample(s.clamp(-1.0, 1.0))));
        while self.pending.len() >= FRAME_SAMPLES {
            let mut frame = [0_i16; FRAME_SAMPLES];
            frame.copy_from_slice(&self.pending[..FRAME_SAMPLES]);
            self.pending.drain(..FRAME_SAMPLES);
            match self.frames.try_send(frame) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => self.pending.clear(),
            }
        }
    }
}

/// The microphone, as the engine reads it.
#[derive(Debug)]
pub struct DeviceCapture {
    frames: Receiver<Frame>,
}

impl Capture for DeviceCapture {
    fn next_frame(&mut self, timeout: Duration) -> Option<Frame> {
        self.frames.recv_timeout(timeout).ok()
    }
}

/// The speaker, as the engine writes it.
pub struct DevicePlayback {
    queue: Arc<Mutex<VecDeque<f32>>>,
    resampler: Resampler,
    limit: usize,
}

impl std::fmt::Debug for DevicePlayback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DevicePlayback")
    }
}

impl Playback for DevicePlayback {
    fn play(&mut self, frame: &Frame) {
        let input: Vec<f32> = frame.iter().map(|s| f32::from_sample(*s)).collect();
        let mut out = Vec::with_capacity(input.len() * 2);
        self.resampler.process(&input, &mut out);
        let mut queue = self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.extend(out);
        let excess = queue.len().saturating_sub(self.limit);
        queue.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_rate_ratio() {
        for (from, to) in [(44_100, 48_000), (48_000, 44_100), (16_000, 48_000), (48_000, 48_000)] {
            let mut resampler = Resampler::new(from, to);
            let mut out = Vec::new();
            let input = vec![0.5_f32; from as usize];
            resampler.process(&input, &mut out);
            let expected = to as usize;
            assert!(out.len().abs_diff(expected) <= 1, "{from}->{to}: {}", out.len());
            assert!(out.iter().skip(4).all(|s| (*s - 0.5).abs() < 1e-6), "{from}->{to}: settles");
        }
    }

    #[test]
    fn the_ring_tone_follows_its_cadence() {
        let mut tone = RingTone::new(8_000);
        let samples: Vec<f32> = (0..8_000 * 6).map(|_| tone.next_sample()).collect();
        let on = &samples[..8_000 * 2];
        let off = &samples[8_000 * 2..];
        assert!(on.iter().any(|s| s.abs() > 0.05));
        assert!(on.iter().all(|s| s.abs() <= RING_LEVEL));
        assert!(off.iter().all(|s| *s == 0.0));
        // The cadence repeats.
        assert_eq!(tone.next_sample(), samples[0]);
    }

    #[test]
    fn capture_cuts_stereo_into_mono_frames() {
        let (tx, rx) = sync_channel(4);
        let mut cutter = Cutter::new(48_000, 2, tx);
        // 30 ms of stereo: left 0.5, right 0.0 -> mono 0.25.
        let data: Vec<f32> = (0..1440).flat_map(|_| [0.5_f32, 0.0]).collect();
        cutter.push(&data);
        let frame = rx.try_recv().expect("one frame");
        assert!(rx.try_recv().is_err());
        assert!(frame[10..].iter().all(|s| (*s - i16::from_sample(0.25_f32)).abs() <= 1));
        assert_eq!(cutter.pending.len(), 480);
    }
}
