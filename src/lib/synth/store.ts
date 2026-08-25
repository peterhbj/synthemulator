import { create } from "zustand";
import {
  notesFromPreset,
  restartArpClock,
  startArpClock,
  stopArpClock,
  type ArpDivision,
  type ArpPattern,
  type ArpPresetId,
} from "./arp";
import {
  DEFAULT_PARAMS,
  enableEngine,
  getEngine,
  type SynthParams,
  type Waveform,
} from "./engine";
import { startWhammyClock, stopWhammyClock } from "./whammy";

export const BEND_RANGES = [2, 7, 12] as const;
export type BendRange = (typeof BEND_RANGES)[number];

type SynthStore = SynthParams & {
  octave: number;
  audioReady: boolean;
  activeNotes: number[];
  heldNotes: number[];
  pedal: boolean;
  bend: number;
  bendRange: BendRange;
  arpOn: boolean;
  arpLatch: boolean;
  arpPattern: ArpPattern;
  arpRate: ArpDivision;
  arpTempo: number;
  arpOctaves: number;
  arpGate: number;
  arpPreset: ArpPresetId | null;
  arpPool: number[];
  whammyOn: boolean;
  whammyStep: number;
  setWaveform: (waveform: Waveform) => void;
  setCutoff: (cutoff: number) => void;
  setResonance: (resonance: number) => void;
  setAttack: (attack: number) => void;
  setDecay: (decay: number) => void;
  setSustainLevel: (sustain: number) => void;
  setRelease: (release: number) => void;
  setVolume: (volume: number) => void;
  setBend: (bend: number) => void;
  setBendRange: (range: BendRange) => void;
  setArpOn: (on: boolean) => void;
  setArpLatch: (on: boolean) => void;
  setArpPattern: (pattern: ArpPattern) => void;
  setArpRate: (rate: ArpDivision) => void;
  setArpTempo: (tempo: number) => void;
  setArpOctaves: (octaves: number) => void;
  setArpGate: (gate: number) => void;
  loadArpPreset: (id: Exclude<ArpPresetId, "live">) => void;
  clearArp: () => void;
  setWhammyOn: (on: boolean) => void;
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

function pushBend(bend: number, range: number) {
  getEngine()?.setBend(bend, range);
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
  bend: 0,
  bendRange: 2,
  arpOn: false,
  arpLatch: true,
  arpPattern: "up",
  arpRate: "8n",
  arpTempo: 125,
  arpOctaves: 1,
  arpGate: 0.62,
  arpPreset: null,
  arpPool: [],
  whammyOn: false,
  whammyStep: 0,

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
  setBend: (bend) => {
    const next = Math.min(1, Math.max(-1, bend));
    pushBend(next, get().bendRange);
    set({ bend: next });
  },
  setBendRange: (range) => {
    pushBend(get().bend, range);
    set({ bendRange: range });
  },

  setArpOn: (on) => {
    if (on) {
      const { heldNotes, arpPool } = get();
      const pool = arpPool.length ? arpPool : [...heldNotes];
      set({
        arpOn: true,
        arpPool: pool,
        arpPreset: pool.length ? get().arpPreset ?? "live" : get().arpPreset,
      });
      startArpClock();
      return;
    }
    stopArpClock();
    getEngine()?.allNotesOff();
    set({ arpOn: false, activeNotes: [] });
  },
  setArpLatch: (on) => {
    if (on) {
      set({ arpLatch: true });
      return;
    }
    const { heldNotes } = get();
    set({
      arpLatch: false,
      arpPool: get().arpPool.filter((n) => heldNotes.includes(n)),
      arpPreset: "live",
    });
  },
  setArpPattern: (pattern) => {
    restartArpClock();
    set({ arpPattern: pattern });
  },
  setArpRate: (rate) => set({ arpRate: rate }),
  setArpTempo: (tempo) => set({ arpTempo: tempo }),
  setArpOctaves: (octaves) => {
    restartArpClock();
    set({ arpOctaves: Math.min(3, Math.max(1, Math.round(octaves))) });
  },
  setArpGate: (gate) => set({ arpGate: Math.min(0.95, Math.max(0.12, gate)) }),
  loadArpPreset: (id) => {
    const pool = notesFromPreset(id, get().octave);
    set({
      arpOn: true,
      arpLatch: true,
      arpPreset: id,
      arpPool: pool,
    });
    restartArpClock();
    startArpClock();
  },
  clearArp: () => {
    stopArpClock();
    getEngine()?.allNotesOff();
    set({
      arpOn: false,
      arpPool: [],
      arpPreset: null,
      activeNotes: [],
    });
  },
  setWhammyOn: (on) => {
    if (on) {
      set({ whammyOn: true });
      startWhammyClock();
      return;
    }
    stopWhammyClock();
    set({ whammyOn: false, whammyStep: 0 });
  },

  shiftOctave: (delta) => {
    const octave = Math.min(MAX_OCTAVE, Math.max(MIN_OCTAVE, get().octave + delta));
    const { arpPreset } = get();
    if (arpPreset && arpPreset !== "live") {
      set({ octave, arpPool: notesFromPreset(arpPreset, octave) });
      restartArpClock();
      return;
    }
    set({ octave });
  },

  enableAudio: async () => {
    const engine = await enableEngine();
    const state = get();
    engine.setParams({
      waveform: state.waveform,
      cutoff: state.cutoff,
      resonance: state.resonance,
      attack: state.attack,
      decay: state.decay,
      sustain: state.sustain,
      release: state.release,
      volume: state.volume,
    });
    engine.setBend(state.bend, state.bendRange);
    set({ audioReady: true });
    if (state.arpOn) startArpClock();
    if (state.whammyOn) startWhammyClock();
  },

  noteOn: (midi) => {
    const { arpOn } = get();
    if (arpOn) {
      set((s) => ({
        heldNotes: s.heldNotes.includes(midi) ? s.heldNotes : [...s.heldNotes, midi],
        arpPool: s.arpPool.includes(midi) ? s.arpPool : [...s.arpPool, midi],
        arpPreset: "live",
      }));
      startArpClock();
      return;
    }
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
    const { arpOn, arpLatch, pedal } = get();
    if (arpOn) {
      set((s) => ({
        heldNotes: s.heldNotes.filter((n) => n !== midi),
        arpPool: arpLatch ? s.arpPool : s.arpPool.filter((n) => n !== midi),
      }));
      return;
    }
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
    const { heldNotes, activeNotes, arpOn } = get();
    if (arpOn) {
      set({ pedal: false });
      return;
    }
    const held = new Set(heldNotes);
    const releasing = activeNotes.filter((n) => !held.has(n));
    for (const midi of releasing) getEngine()?.noteOff(midi);
    set({
      pedal: false,
      activeNotes: activeNotes.filter((n) => held.has(n)),
    });
  },

  panic: () => {
    stopArpClock();
    getEngine()?.allNotesOff();
    getEngine()?.setBend(0, get().bendRange);
    set({
      activeNotes: [],
      heldNotes: [],
      pedal: false,
      bend: 0,
      arpPool:
        get().arpOn && get().arpPreset && get().arpPreset !== "live"
          ? get().arpPool
          : [],
    });
    if (get().arpOn && get().arpPool.length) startArpClock();
    if (get().whammyOn) startWhammyClock();
  },
}));
