use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Sample, SampleFormat, StreamConfig};

use super::ring::AudioRing;

pub struct GuitarInput {
    ring: Arc<AudioRing>,
    peak: Arc<AtomicU32>,
    out_sr: f32,
    _stream: Option<cpal::Stream>,
    pub devices: Vec<String>,
    pub connected: Option<String>,
    pub hold_off: bool,
}

impl GuitarInput {
    pub fn new(ring: Arc<AudioRing>, peak: Arc<AtomicU32>, out_sr: f32) -> Self {
        let mut g = Self {
            ring,
            peak,
            out_sr,
            _stream: None,
            devices: Vec::new(),
            connected: None,
            hold_off: false,
        };
        g.refresh();
        g
    }

    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    pub fn refresh(&mut self) {
        self.devices = list_input_devices();
        if let Some(name) = &self.connected {
            if !self.devices.iter().any(|d| d == name) {
                self.disconnect_silent();
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.hold_off = true;
        self.disconnect_silent();
    }

    fn disconnect_silent(&mut self) {
        self._stream = None;
        self.connected = None;
    }

    pub fn auto_connect(&mut self) -> Result<(), String> {
        if self.hold_off || self.connected.is_some() {
            return Ok(());
        }
        self.devices = list_input_devices();
        if let Some(name) = pick_guitar_device(&self.devices) {
            self.connect(&name)
        } else {
            Ok(())
        }
    }

    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        self.hold_off = false;
        self._stream = None;
        self.connected = None;

        let host = cpal::default_host();
        let device = host
            .input_devices()
            .map_err(|e| format!("input devices: {e}"))?
            .find(|d| d.name().ok().as_deref() == Some(name))
            .ok_or_else(|| format!("entrada não encontrada: {name}"))?;

        let supported = device
            .default_input_config()
            .map_err(|e| format!("input config: {e}"))?;
        let mut config: StreamConfig = supported.config();
        config.buffer_size = BufferSize::Fixed(256);
        // Prefer the output graph rate so PipeWire resamples the GT-100
        // (44.1) onto the same clock instead of Helix RateConv drifting.
        let want = cpal::SampleRate(self.out_sr.round().max(1.0) as u32);
        if config.sample_rate != want {
            config.sample_rate = want;
        }
        let ring = self.ring.clone();
        let peak = self.peak.clone();
        let out_sr = self.out_sr;

        let stream = match open_input(&device, &supported, &config, out_sr, ring.clone(), peak.clone()) {
            Ok(s) => s,
            Err(_) => {
                let mut native = supported.config();
                native.buffer_size = BufferSize::Fixed(256);
                open_input(&device, &supported, &native, out_sr, ring, peak)?
            }
        };
        stream.play().map_err(|e| format!("guitar stream: {e}"))?;
        self.connected = Some(name.to_string());
        self._stream = Some(stream);
        Ok(())
    }
}

fn open_input(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    config: &StreamConfig,
    out_sr: f32,
    ring: Arc<AudioRing>,
    peak: Arc<AtomicU32>,
) -> Result<cpal::Stream, String> {
    let in_sr = config.sample_rate.0 as f32;
    let channels = config.channels as usize;
    match supported.sample_format() {
        SampleFormat::F32 => build_input::<f32>(device, config, channels, in_sr, out_sr, ring, peak),
        SampleFormat::I16 => build_input::<i16>(device, config, channels, in_sr, out_sr, ring, peak),
        SampleFormat::U16 => build_input::<u16>(device, config, channels, in_sr, out_sr, ring, peak),
        other => Err(format!("formato de entrada: {other}")),
    }
}

fn build_input<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    in_sr: f32,
    out_sr: f32,
    ring: Arc<AudioRing>,
    peak: Arc<AtomicU32>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + Sample,
    f32: cpal::FromSample<T>,
{
    let err_fn = |e| eprintln!("helix guitar: {e}");
    let conv = RateConv::new(in_sr, out_sr);
    let mut try_cfg = config.clone();
    let make = |cfg: &StreamConfig| {
        let ring = ring.clone();
        let peak = peak.clone();
        let mut conv = conv.clone();
        let mut env = 0.0f32;
        device.build_input_stream(
            cfg,
            move |data: &[T], _| {
                let ch = channels.max(1);
                for frame in data.chunks(ch) {
                    let mut sum = 0.0;
                    let n = frame.len().min(2).max(1);
                    for s in frame.iter().take(n) {
                        sum += (*s).to_sample::<f32>();
                    }
                    let mono = sum / n as f32;
                    env = env.max(mono.abs());
                    conv.push(mono, &ring);
                }
                env *= 0.85;
                peak.store(env.to_bits(), Ordering::Relaxed);
            },
            err_fn,
            None,
        )
    };
    match make(&try_cfg) {
        Ok(s) => Ok(s),
        Err(_) => {
            try_cfg.buffer_size = BufferSize::Default;
            make(&try_cfg).map_err(|e| format!("open guitar input: {e}"))
        }
    }
}

#[derive(Clone)]
struct RateConv {
    err: f32,
    in_sr: f32,
    out_sr: f32,
}

impl RateConv {
    fn new(in_sr: f32, out_sr: f32) -> Self {
        Self {
            err: 0.0,
            in_sr: in_sr.max(1.0),
            out_sr: out_sr.max(1.0),
        }
    }

    fn push(&mut self, x: f32, ring: &AudioRing) {
        self.err += self.out_sr;
        while self.err >= self.in_sr {
            self.err -= self.in_sr;
            ring.push(x);
        }
    }
}

pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devs) = host.input_devices() else {
        return Vec::new();
    };
    devs.filter_map(|d| d.name().ok()).collect()
}

pub fn pick_guitar_device(devices: &[String]) -> Option<String> {
    devices
        .iter()
        .max_by_key(|d| score_guitar_device(d))
        .filter(|d| score_guitar_device(d) >= 40)
        .cloned()
}

pub fn score_guitar_device(name: &str) -> i32 {
    let l = name.to_ascii_lowercase();
    if l.contains("monitor")
        || l.contains("hdmi")
        || l.contains("headset")
        || l.contains("dmic")
        || l.contains("mic1")
        || l.contains("webcam")
    {
        return -100;
    }
    let mut s = 0;
    if l.contains("gt-100") || l.contains("gt100") {
        s += 80;
    }
    if l.contains("boss") {
        s += 50;
    }
    if l.contains("roland") {
        s += 30;
    }
    if l.contains("guitar") {
        s += 40;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_gt100() {
        let name = pick_guitar_device(&[
            "Headset Microphone".into(),
            "GT-100 Analog Surround 4.0".into(),
            "Built-in Mic".into(),
        ]);
        assert!(name.unwrap().contains("GT-100"));
    }
}
