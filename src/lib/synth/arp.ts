import { getEngine } from "./engine";
import { octaveBaseMidi } from "./notes";
import { useSynth } from "./store";

export type ArpPattern = "up" | "down" | "updown" | "random" | "order";
export type ArpDivision = "4n" | "8n" | "8t" | "16n" | "16t";
export type ArpPresetId =
  | "live"
  | "major"
  | "minor"
  | "maj7"
  | "min7"
  | "fifths"
  | "octaves"
  | "sus"
  | "cascade";

export const ARP_PATTERNS: { id: ArpPattern; label: string }[] = [
  { id: "up", label: "Up" },
  { id: "down", label: "Down" },
  { id: "updown", label: "Up/Dn" },
  { id: "random", label: "Rand" },
  { id: "order", label: "Order" },
];

export const ARP_RATES: { id: ArpDivision; label: string }[] = [
  { id: "4n", label: "1/4" },
  { id: "8n", label: "1/8" },
  { id: "8t", label: "1/8T" },
  { id: "16n", label: "1/16" },
  { id: "16t", label: "1/16T" },
];

export const ARP_PRESETS: {
  id: Exclude<ArpPresetId, "live">;
  label: string;
  intervals: number[];
}[] = [
  { id: "major", label: "Major", intervals: [0, 4, 7, 12] },
  { id: "minor", label: "Minor", intervals: [0, 3, 7, 12] },
  { id: "maj7", label: "Maj7", intervals: [0, 4, 7, 11] },
  { id: "min7", label: "Min7", intervals: [0, 3, 7, 10] },
  { id: "fifths", label: "Fifths", intervals: [0, 7, 12, 19] },
  { id: "octaves", label: "Octaves", intervals: [0, 12, 24, 12] },
  { id: "sus", label: "Sus", intervals: [0, 5, 7, 12] },
  { id: "cascade", label: "Cascade", intervals: [0, 4, 7, 11, 14, 19] },
];

export function notesFromPreset(
  id: Exclude<ArpPresetId, "live">,
  octave: number,
): number[] {
  const preset = ARP_PRESETS.find((p) => p.id === id);
  if (!preset) return [];
  const root = octaveBaseMidi(octave);
  return preset.intervals.map((interval) => root + interval);
}

function stepSeconds(bpm: number, division: ArpDivision): number {
  const beat = 60 / Math.max(40, bpm);
  switch (division) {
    case "4n":
      return beat;
    case "8n":
      return beat / 2;
    case "8t":
      return beat / 3;
    case "16n":
      return beat / 4;
    case "16t":
      return beat / 6;
  }
}

function expandOctaves(notes: number[], octaves: number): number[] {
  const out = [...notes];
  for (let octave = 1; octave < octaves; octave++) {
    for (const note of notes) out.push(note + 12 * octave);
  }
  return out;
}

export function buildArpSequence(
  notes: number[],
  pattern: ArpPattern,
  octaves: number,
): number[] {
  if (notes.length === 0) return [];
  if (pattern === "order") return expandOctaves(notes, octaves);
  const unique = [...new Set(notes)].sort((a, b) => a - b);
  const expanded = expandOctaves(unique, octaves);
  if (pattern === "down") return [...expanded].reverse();
  if (pattern === "updown") {
    if (expanded.length < 2) return expanded;
    return [...expanded, ...expanded.slice(1, -1).reverse()];
  }
  return expanded;
}

let timer: ReturnType<typeof setInterval> | null = null;
let nextTime = 0;
let stepIndex = 0;
let sounding: number | null = null;
let pulseToken = 0;

function silenceCurrent(): void {
  pulseToken += 1;
  if (sounding === null) return;
  getEngine()?.noteOff(sounding);
  const midi = sounding;
  sounding = null;
  useSynth.setState((s) => ({
    activeNotes: s.activeNotes.filter((n) => n !== midi),
  }));
}

function playStep(): void {
  const engine = getEngine();
  const state = useSynth.getState();
  if (!engine || !state.arpOn) return;

  const sequence = buildArpSequence(state.arpPool, state.arpPattern, state.arpOctaves);
  if (sequence.length === 0) {
    silenceCurrent();
    return;
  }

  let midi: number;
  if (state.arpPattern === "random") {
    let pick = sequence[Math.floor(Math.random() * sequence.length)] ?? sequence[0];
    if (sequence.length > 1 && pick === sounding) {
      pick = sequence[(sequence.indexOf(pick) + 1) % sequence.length] ?? pick;
    }
    midi = pick;
  } else {
    midi = sequence[stepIndex % sequence.length] ?? sequence[0];
    stepIndex += 1;
  }

  if (sounding !== null && sounding !== midi) {
    engine.noteOff(sounding, 0.02);
  }
  engine.noteOn(midi);
  sounding = midi;
  useSynth.setState({ activeNotes: [midi] });

  const step = stepSeconds(state.arpTempo, state.arpRate);
  const token = ++pulseToken;
  window.setTimeout(() => {
    if (token !== pulseToken) return;
    if (sounding !== midi) return;
    engine.noteOff(midi);
    sounding = null;
    useSynth.setState((s) => ({
      activeNotes: s.activeNotes.filter((n) => n !== midi),
    }));
  }, Math.max(20, step * state.arpGate * 1000));
}

function pulse(): void {
  const engine = getEngine();
  const state = useSynth.getState();
  if (!engine || !state.arpOn || engine.ctx.state !== "running") return;

  const now = engine.ctx.currentTime;
  const step = stepSeconds(state.arpTempo, state.arpRate);
  if (nextTime === 0) nextTime = now;
  if (now + 0.008 < nextTime) return;

  playStep();
  nextTime += step;
  if (nextTime < now) nextTime = now + step;
}

export function startArpClock(): void {
  if (timer !== null) return;
  nextTime = 0;
  stepIndex = 0;
  timer = setInterval(pulse, 12);
}

export function stopArpClock(): void {
  if (timer !== null) {
    clearInterval(timer);
    timer = null;
  }
  nextTime = 0;
  stepIndex = 0;
  silenceCurrent();
}

export function restartArpClock(): void {
  stepIndex = 0;
  nextTime = 0;
}

export function isArpClockRunning(): boolean {
  return timer !== null;
}
