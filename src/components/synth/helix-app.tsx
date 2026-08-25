import { Minus, Plus, Volume2 } from "lucide-react";
import { AuthSlot } from "@/components/auth-slot";
import { ArpPanel } from "@/components/synth/arp-panel";
import { EnableOverlay } from "@/components/synth/enable-overlay";
import { Keyboard } from "@/components/synth/keyboard";
import { Knob } from "@/components/synth/knob";
import { PitchWheel } from "@/components/synth/pitch-wheel";
import { Oscilloscope } from "@/components/synth/oscilloscope";
import { WaveformSelect } from "@/components/synth/waveform-select";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { midiToName, octaveBaseMidi, VISIBLE_SEMITONES } from "@/lib/synth/notes";
import { MAX_OCTAVE, MIN_OCTAVE, useSynth } from "@/lib/synth/store";

function formatHz(value: number) {
  return value >= 1000 ? `${(value / 1000).toFixed(1)} kHz` : `${Math.round(value)} Hz`;
}

function formatMs(value: number) {
  return value < 1 ? `${Math.round(value * 1000)} ms` : `${value.toFixed(2)} s`;
}

function formatPct(value: number) {
  return `${Math.round(value * 100)}%`;
}

export function HelixApp() {
  const waveform = useSynth((s) => s.waveform);
  const cutoff = useSynth((s) => s.cutoff);
  const resonance = useSynth((s) => s.resonance);
  const attack = useSynth((s) => s.attack);
  const decay = useSynth((s) => s.decay);
  const sustain = useSynth((s) => s.sustain);
  const release = useSynth((s) => s.release);
  const volume = useSynth((s) => s.volume);
  const octave = useSynth((s) => s.octave);
  const audioReady = useSynth((s) => s.audioReady);
  const pedal = useSynth((s) => s.pedal);
  const arpOn = useSynth((s) => s.arpOn);
  const whammyOn = useSynth((s) => s.whammyOn);
  const setWaveform = useSynth((s) => s.setWaveform);
  const setCutoff = useSynth((s) => s.setCutoff);
  const setResonance = useSynth((s) => s.setResonance);
  const setAttack = useSynth((s) => s.setAttack);
  const setDecay = useSynth((s) => s.setDecay);
  const setSustainLevel = useSynth((s) => s.setSustainLevel);
  const setRelease = useSynth((s) => s.setRelease);
  const setVolume = useSynth((s) => s.setVolume);
  const shiftOctave = useSynth((s) => s.shiftOctave);

  const low = midiToName(octaveBaseMidi(octave));
  const high = midiToName(octaveBaseMidi(octave) + VISIBLE_SEMITONES);

  return (
    <div className="flex min-h-dvh flex-col bg-bg text-fg">
      <header className="flex items-center justify-between gap-4 px-4 py-4 sm:px-6">
        <div className="min-w-0">
          <p className="text-2xs font-medium uppercase tracking-[0.22em] text-subtle">
            Analog keyboard
          </p>
          <h1 className="text-xl font-semibold tracking-tight sm:text-2xl">Helix</h1>
        </div>
        <AuthSlot />
      </header>

      <main className="mx-auto flex w-full max-w-6xl flex-1 flex-col gap-5 px-4 pb-8 sm:px-6">
        <section className="relative rounded-3xl bg-surface p-3 shadow-[var(--shadow-panel)] sm:p-4">
          <EnableOverlay />

          <div className="flex flex-col gap-5 lg:grid lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1.4fr)] lg:items-stretch lg:gap-6">
            <Oscilloscope />

            <div className="flex flex-col gap-5">
              <WaveformSelect
                value={waveform}
                onChange={setWaveform}
                disabled={!audioReady}
              />

              <div className="grid gap-5 sm:grid-cols-2">
                <fieldset className="flex flex-col gap-3">
                  <legend className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
                    Filter
                  </legend>
                  <div className="flex justify-around gap-2">
                    <Knob
                      label="Cutoff"
                      value={cutoff}
                      min={80}
                      max={12000}
                      logarithmic
                      onChange={setCutoff}
                      format={formatHz}
                      disabled={!audioReady}
                    />
                    <Knob
                      label="Reso"
                      value={resonance}
                      min={0.1}
                      max={18}
                      step={0.1}
                      onChange={setResonance}
                      format={(v) => v.toFixed(1)}
                      disabled={!audioReady}
                    />
                  </div>
                </fieldset>

                <fieldset className="flex flex-col gap-3">
                  <legend className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
                    Envelope
                  </legend>
                  <div className="flex justify-around gap-1 sm:gap-2">
                    <Knob
                      label="A"
                      value={attack}
                      min={0.005}
                      max={2}
                      logarithmic
                      onChange={setAttack}
                      format={formatMs}
                      disabled={!audioReady}
                    />
                    <Knob
                      label="D"
                      value={decay}
                      min={0.01}
                      max={2}
                      logarithmic
                      onChange={setDecay}
                      format={formatMs}
                      disabled={!audioReady}
                    />
                    <Knob
                      label="S"
                      value={sustain}
                      min={0}
                      max={1}
                      step={0.01}
                      onChange={setSustainLevel}
                      format={formatPct}
                      disabled={!audioReady}
                    />
                    <Knob
                      label="R"
                      value={release}
                      min={0.02}
                      max={3}
                      logarithmic
                      onChange={setRelease}
                      format={formatMs}
                      disabled={!audioReady}
                    />
                  </div>
                </fieldset>
              </div>
            </div>
          </div>

          <div className="mt-5">
            <ArpPanel />
          </div>

          <div className="mt-5 flex flex-col gap-4 border-t border-border/80 pt-4 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="secondary"
                size="icon-sm"
                aria-label="Octave down"
                disabled={octave <= MIN_OCTAVE}
                onClick={() => shiftOctave(-1)}
              >
                <Minus className="size-4" />
              </Button>
              <div className="min-w-24 text-center">
                <p className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
                  Octave
                </p>
                <p className="font-mono text-sm tabular-nums text-fg">
                  {low}–{high}
                </p>
              </div>
              <Button
                type="button"
                variant="secondary"
                size="icon-sm"
                aria-label="Octave up"
                disabled={octave >= MAX_OCTAVE}
                onClick={() => shiftOctave(1)}
              >
                <Plus className="size-4" />
              </Button>
              <span
                className={`ml-2 rounded-full px-2 py-1 text-2xs font-medium uppercase tracking-[0.12em] ${
                  pedal ? "bg-accent text-accent-fg" : "bg-elevated text-subtle"
                }`}
              >
                Sustain
              </span>
              <span
                className={`rounded-full px-2 py-1 text-2xs font-medium uppercase tracking-[0.12em] ${
                  arpOn ? "bg-accent text-accent-fg" : "bg-elevated text-subtle"
                }`}
              >
                Arp
              </span>
              <span
                className={`rounded-full px-2 py-1 text-2xs font-medium uppercase tracking-[0.12em] ${
                  whammyOn ? "bg-accent text-accent-fg" : "bg-elevated text-subtle"
                }`}
              >
                Whammy
              </span>
            </div>

            <div className="flex min-w-0 items-center gap-3 sm:w-64">
              <Volume2 className="size-4 shrink-0 text-muted" aria-hidden />
              <Slider
                min={0}
                max={1}
                step={0.01}
                value={[volume]}
                onValueChange={(v) => setVolume(v[0] ?? 0)}
                disabled={!audioReady}
                aria-label="Volume"
              />
              <span className="w-10 text-right font-mono text-2xs tabular-nums text-muted">
                {formatPct(volume)}
              </span>
            </div>
          </div>

          <div className="mt-4 flex items-stretch gap-3">
            <PitchWheel />
            <div className="min-w-0 flex-1">
              <Keyboard />
            </div>
          </div>
        </section>

        <p className="px-1 text-center text-xs text-subtle sm:text-left">
          Whammy loops the Map of the Problematique octave pattern (−1 / 0 / +1)
          on 16ths. Pair it with a loop and twist the filter. Latch holds your
          chord. Drag the bend wheel or hold up/down arrows. Z and Q rows play
          notes, Space sustains, [ ] shifts octave, Esc silences all.
        </p>
      </main>
    </div>
  );
}
