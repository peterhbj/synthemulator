export type Waveform = "sine" | "square" | "sawtooth" | "triangle";

export const NOTE_NAMES = [
  "C",
  "C#",
  "D",
  "D#",
  "E",
  "F",
  "F#",
  "G",
  "G#",
  "A",
  "A#",
  "B",
] as const;

export const WHITE_PCS = [0, 2, 4, 5, 7, 9, 11] as const;
export const BLACK_PCS = [1, 3, 6, 8, 10] as const;

/** Computer-key → semitone offset from the current octave's C. */
export const COMPUTER_KEY_MAP: Record<string, number> = {
  z: 0,
  s: 1,
  x: 2,
  d: 3,
  c: 4,
  v: 5,
  g: 6,
  b: 7,
  h: 8,
  n: 9,
  j: 10,
  m: 11,
  ",": 12,
  l: 13,
  ".": 14,
  ";": 15,
  "/": 16,
  q: 12,
  "2": 13,
  w: 14,
  "3": 15,
  e: 16,
  r: 17,
  "5": 18,
  t: 19,
  "6": 20,
  y: 21,
  "7": 22,
  u: 23,
  i: 24,
  "9": 25,
  o: 26,
  "0": 27,
  p: 28,
};

/** Inverse: midi offset → preferred computer-key label for the on-screen keys. */
export const OFFSET_KEY_LABEL: Record<number, string> = {
  0: "Z",
  1: "S",
  2: "X",
  3: "D",
  4: "C",
  5: "V",
  6: "G",
  7: "B",
  8: "H",
  9: "N",
  10: "J",
  11: "M",
  12: "Q",
  13: "2",
  14: "W",
  15: "3",
  16: "E",
  17: "R",
  18: "5",
  19: "T",
  20: "6",
  21: "Y",
  22: "7",
  23: "U",
  24: "I",
};

export const VISIBLE_SEMITONES = 24;

export function midiToHz(midi: number): number {
  return 440 * 2 ** ((midi - 69) / 12);
}

export function midiToName(midi: number): string {
  const pc = ((midi % 12) + 12) % 12;
  const octave = Math.floor(midi / 12) - 1;
  return `${NOTE_NAMES[pc]}${octave}`;
}

export function isBlackKey(midi: number): boolean {
  const pc = ((midi % 12) + 12) % 12;
  return (BLACK_PCS as readonly number[]).includes(pc);
}

export function octaveBaseMidi(octave: number): number {
  return (octave + 1) * 12;
}

export function visibleMidis(octave: number): number[] {
  const base = octaveBaseMidi(octave);
  return Array.from({ length: VISIBLE_SEMITONES + 1 }, (_, i) => base + i);
}

export function whiteKeyIndex(midi: number, baseMidi: number): number {
  let index = 0;
  for (let m = baseMidi; m < midi; m++) {
    if (!isBlackKey(m)) index += 1;
  }
  return index;
}

export function countWhiteKeys(midis: number[]): number {
  return midis.filter((m) => !isBlackKey(m)).length;
}
