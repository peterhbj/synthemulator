use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, StreamConfig};
use crossbeam_channel::{bounded, Receiver, Sender};

use super::engine::{Engine, Snapshot};
use super::gt100::GtCmd;
use super::ring::AudioRing;
use crate::synth::Command;

pub type AudioError = String;

pub struct AudioHost {
    tx: Sender<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
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
            )?,
            other => {
                return Err(format!("unsupported sample format: {other}"));
            }
        };

        stream.play().map_err(|e| format!("stream play: {e}"))?;

        Ok(Self {
            tx,
            snapshot,
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
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    sample_rate: f32,
    rx: Receiver<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    guitar_ring: Arc<AudioRing>,
    gt_tx: Sender<GtCmd>,
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
        device.build_output_stream(
            cfg,
            move |data: &mut [T], _| {
                while let Ok(cmd) = rx.try_recv() {
                    engine.handle(cmd);
                }
                for frame in data.chunks_mut(channels.max(1)) {
                    let s = engine.render();
                    let sample = T::from_sample(s);
                    for slot in frame.iter_mut() {
                        *slot = sample;
                    }
                }
                if let Ok(mut g) = snapshot.try_lock() {
                    *g = engine.snapshot();
                }
            },
            err_fn,
            None,
        )
    };

    open_low_latency(config.clone(), make).map_err(|e| format!("open stream: {e}"))
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
