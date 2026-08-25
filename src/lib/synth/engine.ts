import { midiToHz, type Waveform } from "./notes";

export type { Waveform } from "./notes";

export type SynthParams = {
  waveform: Waveform;
  cutoff: number;
  resonance: number;
  attack: number;
  decay: number;
  sustain: number;
  release: number;
  volume: number;
};

export const DEFAULT_PARAMS: SynthParams = {
  waveform: "sawtooth",
  cutoff: 2200,
  resonance: 1.8,
  attack: 0.018,
  decay: 0.16,
  sustain: 0.62,
  release: 0.28,
  volume: 0.72,
};

type Voice = {
  midi: number;
  osc: OscillatorNode;
  gain: GainNode;
  startedAt: number;
  stopping: boolean;
};

const MAX_VOICES = 12;
const VOICE_GAIN = 0.16;
const MIN_ENV = 0.004;

export class SynthEngine {
  readonly ctx: AudioContext;
  readonly analyser: AnalyserNode;
  private readonly filter: BiquadFilterNode;
  private readonly master: GainNode;
  private readonly voices = new Map<number, Voice>();
  private params: SynthParams = { ...DEFAULT_PARAMS };
  private bendAmount = 0;
  private bendRange = 2;
  private octaveShift = 0;

  constructor() {
    const ctx = new AudioContext();
    const filter = ctx.createBiquadFilter();
    filter.type = "lowpass";
    filter.frequency.value = this.params.cutoff;
    filter.Q.value = this.params.resonance;

    const compressor = ctx.createDynamicsCompressor();
    compressor.threshold.value = -14;
    compressor.knee.value = 10;
    compressor.ratio.value = 3.5;
    compressor.attack.value = 0.004;
    compressor.release.value = 0.12;

    const master = ctx.createGain();
    master.gain.value = this.params.volume;

    const analyser = ctx.createAnalyser();
    analyser.fftSize = 2048;
    analyser.smoothingTimeConstant = 0.35;

    filter.connect(compressor);
    compressor.connect(master);
    master.connect(analyser);
    analyser.connect(ctx.destination);

    this.ctx = ctx;
    this.filter = filter;
    this.master = master;
    this.analyser = analyser;
  }

  async resume(): Promise<void> {
    if (this.ctx.state === "suspended") {
      await this.ctx.resume();
    }
  }

  setParams(next: Partial<SynthParams>): void {
    this.params = { ...this.params, ...next };
    const now = this.ctx.currentTime;
    if (next.cutoff !== undefined) {
      this.filter.frequency.setTargetAtTime(next.cutoff, now, 0.03);
    }
    if (next.resonance !== undefined) {
      this.filter.Q.setTargetAtTime(next.resonance, now, 0.03);
    }
    if (next.volume !== undefined) {
      this.master.gain.setTargetAtTime(next.volume, now, 0.02);
    }
    if (next.waveform !== undefined) {
      for (const voice of this.voices.values()) {
        if (!voice.stopping) voice.osc.type = next.waveform;
      }
    }
  }

  setBend(amount: number, range = this.bendRange): void {
    this.bendAmount = Math.min(1, Math.max(-1, amount));
    this.bendRange = range;
    this.applyDetune(true);
  }

  setOctaveShift(semitones: number): void {
    this.octaveShift = semitones;
    this.applyDetune(false);
  }

  private totalCents(): number {
    return this.bendAmount * this.bendRange * 100 + this.octaveShift * 100;
  }

  private applyDetune(smooth: boolean): void {
    const cents = this.totalCents();
    const now = this.ctx.currentTime;
    for (const voice of this.voices.values()) {
      if (smooth) voice.osc.detune.setTargetAtTime(cents, now, 0.008);
      else voice.osc.detune.setValueAtTime(cents, now);
    }
  }

  noteOn(midi: number): void {
    if (this.ctx.state !== "running") return;
    const existing = this.voices.get(midi);
    if (existing) {
      this.releaseVoice(existing, 0.012);
      this.voices.delete(midi);
    }
    if (this.voices.size >= MAX_VOICES) this.stealOldest();

    const now = this.ctx.currentTime;
    const { attack, decay, sustain, waveform } = this.params;
    const atk = Math.max(MIN_ENV, attack);
    const dec = Math.max(MIN_ENV, decay);
    const peak = VOICE_GAIN;
    const sus = Math.max(0.0001, peak * sustain);

    const osc = this.ctx.createOscillator();
    osc.type = waveform;
    osc.frequency.setValueAtTime(midiToHz(midi), now);
    osc.detune.setValueAtTime(this.totalCents(), now);

    const gain = this.ctx.createGain();
    gain.gain.setValueAtTime(0.0001, now);
    gain.gain.exponentialRampToValueAtTime(peak, now + atk);
    gain.gain.exponentialRampToValueAtTime(sus, now + atk + dec);

    osc.connect(gain);
    gain.connect(this.filter);
    osc.start(now);

    this.voices.set(midi, { midi, osc, gain, startedAt: now, stopping: false });
  }

  noteOff(midi: number, releaseOverride?: number): void {
    const voice = this.voices.get(midi);
    if (!voice || voice.stopping) return;
    this.releaseVoice(voice, releaseOverride ?? this.params.release);
  }

  allNotesOff(): void {
    for (const midi of [...this.voices.keys()]) {
      this.noteOff(midi, 0.04);
    }
  }

  dispose(): void {
    this.allNotesOff();
    void this.ctx.close();
  }

  private stealOldest(): void {
    let oldest: Voice | undefined;
    for (const voice of this.voices.values()) {
      if (!oldest || voice.startedAt < oldest.startedAt) oldest = voice;
    }
    if (oldest) this.noteOff(oldest.midi, 0.02);
  }

  private releaseVoice(voice: Voice, releaseTime: number): void {
    if (voice.stopping) return;
    voice.stopping = true;
    const now = this.ctx.currentTime;
    const release = Math.max(MIN_ENV, releaseTime);
    const current = Math.max(voice.gain.gain.value, 0.0001);
    voice.gain.gain.cancelScheduledValues(now);
    voice.gain.gain.setValueAtTime(current, now);
    voice.gain.gain.exponentialRampToValueAtTime(0.0001, now + release);
    try {
      voice.osc.stop(now + release + 0.03);
    } catch {
      /* already stopped */
    }
    window.setTimeout(() => {
      try {
        voice.osc.disconnect();
        voice.gain.disconnect();
      } catch {
        /* already disconnected */
      }
      if (this.voices.get(voice.midi) === voice) {
        this.voices.delete(voice.midi);
      }
    }, (release + 0.05) * 1000);
  }
}

let engine: SynthEngine | null = null;

export function getEngine(): SynthEngine | null {
  return engine;
}

export async function enableEngine(): Promise<SynthEngine> {
  if (!engine) engine = new SynthEngine();
  await engine.resume();
  return engine;
}
