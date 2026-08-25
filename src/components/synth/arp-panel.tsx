import type { ReactNode } from "react";
import { Knob } from "@/components/synth/knob";
import { Button } from "@/components/ui/button";
import {
  ARP_PATTERNS,
  ARP_PRESETS,
  ARP_RATES,
} from "@/lib/synth/arp";
import { useSynth } from "@/lib/synth/store";
import { WHAMMY_SEQUENCE } from "@/lib/synth/whammy";
import { cn } from "@/lib/utils";

function Chip({
  active,
  disabled,
  onClick,
  children,
}: {
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "h-8 rounded-lg px-2.5 text-2xs font-medium uppercase tracking-[0.08em] transition-[background-color,color] duration-150",
        active
          ? "bg-accent text-accent-fg"
          : "bg-elevated text-muted shadow-[var(--shadow-panel)] hover:text-fg",
      )}
    >
      {children}
    </button>
  );
}

function WhammyStrip() {
  const on = useSynth((s) => s.whammyOn);
  const step = useSynth((s) => s.whammyStep);

  return (
    <div
      className="flex h-10 gap-0.5"
      role="img"
      aria-label="Map of the Problematique octave sequence, 16 steps"
    >
      {WHAMMY_SEQUENCE.map((semitones, index) => {
        const current = on && index === step;
        return (
          <span
            key={index}
            className={cn(
              "relative min-w-0 flex-1 rounded-sm bg-surface",
              current && "bg-accent",
            )}
          >
            <span
              className={cn(
                "absolute left-1/2 h-1 w-1.5 -translate-x-1/2 rounded-full",
                current ? "bg-accent-fg" : "bg-muted",
                semitones > 0 && "top-0.5",
                semitones === 0 && "top-1/2 -translate-y-1/2",
                semitones < 0 && "bottom-0.5",
              )}
            />
          </span>
        );
      })}
    </div>
  );
}

export function ArpPanel() {
  const audioReady = useSynth((s) => s.audioReady);
  const arpOn = useSynth((s) => s.arpOn);
  const arpLatch = useSynth((s) => s.arpLatch);
  const arpPattern = useSynth((s) => s.arpPattern);
  const arpRate = useSynth((s) => s.arpRate);
  const arpTempo = useSynth((s) => s.arpTempo);
  const arpOctaves = useSynth((s) => s.arpOctaves);
  const arpGate = useSynth((s) => s.arpGate);
  const arpPreset = useSynth((s) => s.arpPreset);
  const whammyOn = useSynth((s) => s.whammyOn);
  const setArpOn = useSynth((s) => s.setArpOn);
  const setArpLatch = useSynth((s) => s.setArpLatch);
  const setArpPattern = useSynth((s) => s.setArpPattern);
  const setArpRate = useSynth((s) => s.setArpRate);
  const setArpTempo = useSynth((s) => s.setArpTempo);
  const setArpOctaves = useSynth((s) => s.setArpOctaves);
  const setArpGate = useSynth((s) => s.setArpGate);
  const loadArpPreset = useSynth((s) => s.loadArpPreset);
  const clearArp = useSynth((s) => s.clearArp);
  const setWhammyOn = useSynth((s) => s.setWhammyOn);
  const enableAudio = useSynth((s) => s.enableAudio);

  const arm = async (run: () => void) => {
    if (!useSynth.getState().audioReady) await enableAudio();
    run();
  };

  return (
    <div className="flex flex-col gap-3 rounded-2xl bg-elevated/60 p-3 shadow-[var(--shadow-panel)]">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
            Arp
          </span>
          <Chip
            active={arpOn}
            onClick={() => void arm(() => setArpOn(!arpOn))}
          >
            {arpOn ? "On" : "Off"}
          </Chip>
          <Chip
            active={arpLatch}
            disabled={!audioReady}
            onClick={() => setArpLatch(!arpLatch)}
          >
            Latch
          </Chip>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={!arpOn && !arpPreset}
          onClick={clearArp}
        >
          Clear
        </Button>
      </div>

      <div className="flex flex-col gap-1.5">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <span className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
              Whammy
            </span>
            <Chip
              active={whammyOn}
              onClick={() => void arm(() => setWhammyOn(!whammyOn))}
            >
              {whammyOn ? "On" : "Off"}
            </Chip>
          </div>
          <span className="font-mono text-2xs tabular-nums text-muted">
            −1 / 0 / +1
          </span>
        </div>
        <WhammyStrip />
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
          Motion
        </span>
        <div className="flex flex-wrap gap-1">
          {ARP_PATTERNS.map((pattern) => (
            <Chip
              key={pattern.id}
              active={arpPattern === pattern.id}
              disabled={!audioReady}
              onClick={() => setArpPattern(pattern.id)}
            >
              {pattern.label}
            </Chip>
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
          Rate
        </span>
        <div className="flex flex-wrap gap-1">
          {ARP_RATES.map((rate) => (
            <Chip
              key={rate.id}
              active={arpRate === rate.id}
              disabled={!audioReady}
              onClick={() => setArpRate(rate.id)}
            >
              {rate.label}
            </Chip>
          ))}
        </div>
      </div>

      <div className="flex justify-around gap-2 pt-1">
        <Knob
          label="Tempo"
          value={arpTempo}
          min={70}
          max={180}
          step={1}
          onChange={setArpTempo}
          format={(v) => `${Math.round(v)}`}
          disabled={!audioReady}
        />
        <Knob
          label="Oct"
          value={arpOctaves}
          min={1}
          max={3}
          step={1}
          onChange={setArpOctaves}
          format={(v) => `${Math.round(v)}`}
          disabled={!audioReady}
        />
        <Knob
          label="Gate"
          value={arpGate}
          min={0.15}
          max={0.95}
          step={0.01}
          onChange={setArpGate}
          format={(v) => `${Math.round(v * 100)}%`}
          disabled={!audioReady}
        />
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
          Loops
        </span>
        <div className="flex flex-wrap gap-1">
          {ARP_PRESETS.map((preset) => (
            <Chip
              key={preset.id}
              active={arpPreset === preset.id}
              onClick={() => void arm(() => loadArpPreset(preset.id))}
            >
              {preset.label}
            </Chip>
          ))}
        </div>
      </div>
    </div>
  );
}
