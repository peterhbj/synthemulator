import { getEngine } from "./engine";
import { useSynth } from "./store";

/** Map of the Problematique octave sequence (16ths, no portamento). */
export const WHAMMY_SEQUENCE = [
  -12, 0, 12, 0, 12, 0, -12, 0, 12, 0, 12, 0, -12, 0, 12, 0,
] as const;

export const WHAMMY_BPM = 125;

let timer: ReturnType<typeof setInterval> | null = null;
let nextTime = 0;
let step = 0;

function sixteenth(bpm: number): number {
  return 60 / Math.max(40, bpm) / 4;
}

function applyStep(index: number): void {
  const semitones = WHAMMY_SEQUENCE[index] ?? 0;
  getEngine()?.setOctaveShift(semitones);
  useSynth.setState({ whammyStep: index });
}

function pulse(): void {
  const engine = getEngine();
  const state = useSynth.getState();
  if (!engine || !state.whammyOn || engine.ctx.state !== "running") return;

  const now = engine.ctx.currentTime;
  const dur = sixteenth(state.arpTempo);
  if (nextTime === 0) nextTime = now;
  if (now + 0.008 < nextTime) return;

  applyStep(step);
  step = (step + 1) % WHAMMY_SEQUENCE.length;
  nextTime += dur;
  if (nextTime < now) nextTime = now + dur;
}

export function startWhammyClock(): void {
  if (timer !== null) return;
  step = 0;
  nextTime = 0;
  applyStep(0);
  timer = setInterval(pulse, 12);
}

export function stopWhammyClock(): void {
  if (timer !== null) {
    clearInterval(timer);
    timer = null;
  }
  nextTime = 0;
  step = 0;
  getEngine()?.setOctaveShift(0);
  useSynth.setState({ whammyStep: 0 });
}

export function isWhammyClockRunning(): boolean {
  return timer !== null;
}
