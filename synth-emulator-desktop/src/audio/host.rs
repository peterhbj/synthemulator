use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, StreamConfig};
use crossbeam_channel::{bounded, Receiver, Sender};

use super::engine::{Engine, Snapshot};
use crate::synth::Command;

pub type AudioError = String;

pub struct AudioHost {
    tx: Sender<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    _stream: cpal::Stream,
    pub sample_rate: f32,
    pub device_name: String,
}

impl AudioHost {
    pub fn start() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no default output device".to_string())?;
        let device_name = device.name().unwrap_or_else(|_| "default".into());
        let supported = device
            .default_output_config()
            .map_err(|e| format!("output config: {e}"))?;

        let mut config: StreamConfig = supported.config();
        config.buffer_size = BufferSize::Fixed(256);

        let sample_rate = config.sample_rate.0 as f32;
        let channels = config.channels as usize;
        let (tx, rx) = bounded::<Command>(1024);
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
            )?,
            SampleFormat::I16 => build_stream::<i16>(
                &device,
                &config,
                channels,
                sample_rate,
                rx,
                snapshot.clone(),
            )?,
            SampleFormat::U16 => build_stream::<u16>(
                &device,
                &config,
                channels,
                sample_rate,
                rx,
                snapshot.clone(),
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
        })
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
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut try_config = config.clone();

    let err_fn = |e| eprintln!("helix audio: {e}");

    let make = |cfg: &StreamConfig| {
        let rx = rx.clone();
        let snapshot = snapshot.clone();
        let mut engine = Engine::new(sample_rate);
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

    match make(&try_config) {
        Ok(stream) => Ok(stream),
        Err(_) => {
            try_config.buffer_size = BufferSize::Default;
            make(&try_config).map_err(|e| format!("open stream: {e}"))
        }
    }
}
