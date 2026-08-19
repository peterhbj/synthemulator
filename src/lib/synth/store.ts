import { create } from "zustand";
import {
  DEFAULT_PARAMS,
  enableEngine,
  getEngine,
  type SynthParams,
  type Waveform,
} from "./engine";

type SynthStore = SynthParams & {
  octave: number;
  audioReady: boolean;
  activeNotes: number[];
  heldNotes: number[];
  pedal: boolean;
  setWaveform: (waveform: Waveform) => void;
  setCutoff: (cutoff: number) => void;
  setResonance: (resonance: number) => void;
  setAttack: (attack: number) => void;
  setDecay: (decay: number) => void;
  setSustainLevel: (sustain: number) => void;
  setRelease: (release: number) => void;
  setVolume: (volume: number) => void;
  shiftOctave: (delta: number) => void;
  enableAudio: () => Promise<void>;
  noteOn: (midi: number) => void;
  noteOff: (midi: number) => void;
  setPedal: (on: boolean) => void;
  panic: () => void;
};

function pushParams(partial: Partial<SynthParams>) {
  getEngine()?.setParams(partial);
}

export const MIN_OCTAVE = 1;
export const MAX_OCTAVE = 6;

export const useSynth = create<SynthStore>((set, get) => ({
  ...DEFAULT_PARAMS,
  octave: 3,
  audioReady: false,
  activeNotes: [],
  heldNotes: [],
  pedal: false,

  setWaveform: (waveform) => {
    pushParams({ waveform });
    set({ waveform });
  },
  setCutoff: (cutoff) => {
    pushParams({ cutoff });
    set({ cutoff });
  },
  setResonance: (resonance) => {
    pushParams({ resonance });
    set({ resonance });
  },
  setAttack: (attack) => {
    pushParams({ attack });
    set({ attack });
  },
  setDecay: (decay) => {
    pushParams({ decay });
    set({ decay });
  },
  setSustainLevel: (sustain) => {
    pushParams({ sustain });
    set({ sustain });
  },
  setRelease: (release) => {
    pushParams({ release });
    set({ release });
  },
  setVolume: (volume) => {
    pushParams({ volume });
    set({ volume });
  },
  shiftOctave: (delta) => {
    set((s) => ({
      octave: Math.min(MAX_OCTAVE, Math.max(MIN_OCTAVE, s.octave + delta)),
    }));
  },

  enableAudio: async () => {
    const engine = await enableEngine();
    const { waveform, cutoff, resonance, attack, decay, sustain, release, volume } =
      get();
    engine.setParams({
      waveform,
      cutoff,
      resonance,
      attack,
      decay,
      sustain,
      release,
      volume,
    });
    set({ audioReady: true });
  },

  noteOn: (midi) => {
    const engine = getEngine();
    if (!engine) return;
    engine.noteOn(midi);
    set((s) => ({
      activeNotes: s.activeNotes.includes(midi)
        ? s.activeNotes
        : [...s.activeNotes, midi],
      heldNotes: s.heldNotes.includes(midi) ? s.heldNotes : [...s.heldNotes, midi],
    }));
  },

  noteOff: (midi) => {
    const { pedal } = get();
    set((s) => ({
      heldNotes: s.heldNotes.filter((n) => n !== midi),
    }));
    if (pedal) return;
    getEngine()?.noteOff(midi);
    set((s) => ({
      activeNotes: s.activeNotes.filter((n) => n !== midi),
    }));
  },

  setPedal: (on) => {
    if (on) {
      set({ pedal: true });
      return;
    }
    const { heldNotes, activeNotes } = get();
    const held = new Set(heldNotes);
    const releasing = activeNotes.filter((n) => !held.has(n));
    for (const midi of releasing) getEngine()?.noteOff(midi);
    set({
      pedal: false,
      activeNotes: activeNotes.filter((n) => held.has(n)),
    });
  },

  panic: () => {
    getEngine()?.allNotesOff();
    set({ activeNotes: [], heldNotes: [], pedal: false });
  },
}));
