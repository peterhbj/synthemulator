import { cn } from "@/lib/utils";
import type { Waveform } from "@/lib/synth/notes";

const WAVES: { id: Waveform; label: string }[] = [
  { id: "sine", label: "Sine" },
  { id: "triangle", label: "Tri" },
  { id: "sawtooth", label: "Saw" },
  { id: "square", label: "Square" },
];

function WaveIcon({ type }: { type: Waveform }) {
  const d =
    type === "sine"
      ? "M2 12 C6 12 6 4 10 4 C14 4 14 20 18 20 C22 20 22 12 26 12"
      : type === "triangle"
        ? "M2 20 L10 4 L18 20 L26 4"
        : type === "sawtooth"
          ? "M2 20 L2 4 L26 20"
          : "M2 20 L2 4 L14 4 L14 20 L26 20 L26 4";
  return (
    <svg viewBox="0 0 28 24" className="h-4 w-6" aria-hidden>
      <path
        d={d}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function WaveformSelect({
  value,
  onChange,
  disabled,
}: {
  value: Waveform;
  onChange: (wave: Waveform) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex flex-col gap-2">
      <span className="text-[0.65rem] font-medium uppercase tracking-[0.14em] text-subtle">
        Wave
      </span>
      <div
        role="radiogroup"
        aria-label="Waveform"
        className="grid grid-cols-4 gap-1 rounded-xl bg-elevated p-1 shadow-[var(--shadow-panel)]"
      >
        {WAVES.map((wave) => {
          const active = wave.id === value;
          return (
            <button
              key={wave.id}
              type="button"
              role="radio"
              aria-checked={active}
              disabled={disabled}
              onClick={() => onChange(wave.id)}
              className={cn(
                "flex min-h-11 flex-col items-center justify-center gap-1 rounded-lg px-1 py-1.5 text-[0.65rem] font-medium uppercase tracking-[0.08em] transition-[background-color,color] duration-150 ease-[cubic-bezier(0.22,1,0.36,1)]",
                active
                  ? "bg-accent text-accent-fg"
                  : "text-muted hover:bg-surface hover:text-fg",
              )}
            >
              <WaveIcon type={wave.id} />
              {wave.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}
