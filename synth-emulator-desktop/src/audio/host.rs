use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, StreamConfig};
use crossbeam_channel::{bounded, Receiver, Sender};

use remix::{RemixCmd, RemixEngine, RemixState, Track};

use super::engine::{Engine, Snapshot};
use super::gt100::GtCmd;
use super::ring::AudioRing;
use crate::synth::Command;

pub type AudioError = String;

pub struct AudioHost {
    tx: Sender<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    remix_tx: Sender<RemixCmd>,
    remix_state: Arc<Mutex<RemixState>>,
    /// Tracks the audio thread replaced; dropped here, not in the callback.
    trash_rx: Receiver<Arc<Track>>,
    _stream: cpal::Stream,
    pub sample_rate: f32,
    pub device_name: String,
    pub guitar_ring: Arc<AudioRing>,
    pub guitar_peak: Arc<AtomicU32>,
    gt_rx: Option<Receiver<GtCmd>>,
}

impl AudioHost {
    pub fn start() -> Result<Self, AudioError> {
        request_pipewire_latency();
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no default output device".to_string())?;
        let device_name = device.name().unwrap_or_else(|_| "default".into());
        let supported = device
            .default_output_config()
            .map_err(|e| format!("output config: {e}"))?;

        let mut config: StreamConfig = supported.config();
        config.buffer_size = BufferSize::Fixed(128);

        let sample_rate = config.sample_rate.0 as f32;
        let channels = config.channels as usize;
        let (tx, rx) = bounded::<Command>(1024);
        let (gt_tx, gt_rx) = bounded::<GtCmd>(64);
        let (remix_tx, remix_rx) = bounded::<RemixCmd>(1024);
        let (trash_tx, trash_rx) = bounded::<Arc<Track>>(64);
        let remix_state = Arc::new(Mutex::new(RemixState::default()));
        let guitar_ring = Arc::new(AudioRing::new(4_096));
        let guitar_peak = Arc::new(AtomicU32::new(0));
        let snapshot = Arc::new(Mutex::new(Snapshot {
            sample_rate,
            ..Snapshot::default()
        }));

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_stream::<f32>(
                &device,
                &config,
                channels,
                sample_rate,
                rx,
                snapshot.clone(),
                guitar_ring.clone(),
                gt_tx.clone(),
                RemixLink {
                    rx: remix_rx.clone(),
                    state: remix_state.clone(),
                    trash: trash_tx.clone(),
                },
            )?,
            SampleFormat::I16 => build_stream::<i16>(
                &device,
                &config,
                channels,
                sample_rate,
                rx,
                snapshot.clone(),
                guitar_ring.clone(),
                gt_tx.clone(),
                RemixLink {
                    rx: remix_rx.clone(),
                    state: remix_state.clone(),
                    trash: trash_tx.clone(),
                },
            )?,
            SampleFormat::U16 => build_stream::<u16>(
                &device,
                &config,
                channels,
                sample_rate,
                rx,
                snapshot.clone(),
                guitar_ring.clone(),
                gt_tx.clone(),
                RemixLink {
                    rx: remix_rx.clone(),
                    state: remix_state.clone(),
                    trash: trash_tx.clone(),
                },
            )?,
            other => {
                return Err(format!("unsupported sample format: {other}"));
            }
        };

        stream.play().map_err(|e| format!("stream play: {e}"))?;

        Ok(Self {
            tx,
            snapshot,
            remix_tx,
            remix_state,
            trash_rx,
            _stream: stream,
            sample_rate,
            device_name,
            guitar_ring,
            guitar_peak,
            gt_rx: Some(gt_rx),
        })
    }

    pub fn take_gt_rx(&mut self) -> Option<Receiver<GtCmd>> {
        self.gt_rx.take()
    }

    pub fn sender(&self) -> Sender<Command> {
        self.tx.clone()
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.try_send(cmd);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn send_remix(&self, cmd: RemixCmd) {
        let _ = self.remix_tx.try_send(cmd);
    }

    pub fn remix_state(&self) -> RemixState {
        self.remix_state.lock().map(|g| *g).unwrap_or_default()
    }

    /// Free tracks the audio thread let go of. Call every frame.
    pub fn drain_trash(&self) {
        while self.trash_rx.try_recv().is_ok() {}
    }
}

/// Channels between the UI and the remix engine on the audio thread.
struct RemixLink {
    rx: Receiver<RemixCmd>,
    state: Arc<Mutex<RemixState>>,
    trash: Sender<Arc<Track>>,
}

#[allow(clippy::too_many_arguments)]
fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    sample_rate: f32,
    rx: Receiver<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    guitar_ring: Arc<AudioRing>,
    gt_tx: Sender<GtCmd>,
    remix_link: RemixLink,
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let err_fn = |e| eprintln!("helix audio: {e}");

    let make = |cfg: &StreamConfig| {
        let rx = rx.clone();
        let snapshot = snapshot.clone();
        let guitar_ring = guitar_ring.clone();
        let gt_tx = gt_tx.clone();
        let mut engine = Engine::with_guitar(sample_rate, guitar_ring, Some(gt_tx));
        let remix_rx = remix_link.rx.clone();
        let remix_state = remix_link.state.clone();
        let trash = remix_link.trash.clone();
        let mut remix = RemixEngine::new(sample_rate);
        device.build_output_stream(
            cfg,
            move |data: &mut [T], _| {
                while let Ok(cmd) = rx.try_recv() {
                    engine.handle(cmd);
                }
                while let Ok(cmd) = remix_rx.try_recv() {
                    if let Some(old) = remix.handle(cmd) {
                        // If the UI is not draining, dropping here is the
                        // lesser evil; the UI normally holds a clone anyway.
                        let _ = trash.try_send(old);
                    }
                }
                for frame in data.chunks_mut(channels.max(1)) {
                    let s = engine.render();
                    let (l, r) = remix.process();
                    let (l, r) = mix_out(s, l, r);
                    write_frame(frame, l, r);
                }
                if let Ok(mut g) = snapshot.try_lock() {
                    *g = engine.snapshot();
                }
                if let Ok(mut g) = remix_state.try_lock() {
                    *g = remix.state();
                }
            },
            err_fn,
            None,
        )
    };

    open_low_latency(config.clone(), make).map_err(|e| format!("open stream: {e}"))
}

/// Synth (mono) on top of the remix bus (stereo). Both are already limited;
/// the sum can still exceed full scale, so it goes through a soft knee.
#[inline]
fn mix_out(synth: f32, l: f32, r: f32) -> (f32, f32) {
    (soft_clip(synth + l), soft_clip(synth + r))
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.9;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        // tanh above the knee, continuous in value and slope, never above 1.
        let over = (a - KNEE) / (1.0 - KNEE);
        (KNEE + (1.0 - KNEE) * over.tanh()).copysign(x)
    }
}

/// Stereo into any channel count: mono gets the average, extra channels
/// repeat the L/R pair.
#[inline]
fn write_frame<T: cpal::SizedSample + cpal::FromSample<f32>>(frame: &mut [T], l: f32, r: f32) {
    if frame.len() == 1 {
        frame[0] = T::from_sample((l + r) * 0.5);
        return;
    }
    for (i, slot) in frame.iter_mut().enumerate() {
        *slot = T::from_sample(if i % 2 == 0 { l } else { r });
    }
}

/// Ask PipeWire for a live-guitar quantum before ALSA streams open.
pub(crate) fn request_pipewire_latency() {
    if std::env::var_os("PIPEWIRE_LATENCY").is_none() {
        std::env::set_var("PIPEWIRE_LATENCY", "128/48000");
    }
}

pub(crate) fn open_low_latency<S, E>(
    mut config: StreamConfig,
    mut make: impl FnMut(&StreamConfig) -> Result<S, E>,
) -> Result<S, E> {
    for frames in [128u32, 64, 256] {
        config.buffer_size = BufferSize::Fixed(frames);
        if let Ok(stream) = make(&config) {
            return Ok(stream);
        }
    }
    config.buffer_size = BufferSize::Default;
    make(&config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_clip_is_transparent_below_the_knee_and_bounded_above() {
        for i in -90..=90 {
            let x = i as f32 / 100.0;
            assert_eq!(soft_clip(x), x);
        }
        for i in 0..1000 {
            let x = i as f32 / 50.0;
            let y = soft_clip(x);
            assert!(x <= 0.9 || (0.9 - 1e-6..=1.0).contains(&y), "{x} -> {y}");
            assert_eq!(soft_clip(-x), -y);
        }
    }

    #[test]
    fn stereo_frames_map_to_channel_counts() {
        let mut mono = [0.0f32; 1];
        write_frame(&mut mono, 0.2, 0.4);
        assert!((mono[0] - 0.3).abs() < 1e-6);
        let mut quad = [0.0f32; 4];
        write_frame(&mut quad, 0.2, 0.4);
        assert_eq!(quad, [0.2, 0.4, 0.2, 0.4]);
    }
}
