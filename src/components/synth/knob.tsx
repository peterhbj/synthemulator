import { useCallback, useId, useRef } from "react";
import { cn } from "@/lib/utils";

type KnobProps = {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
  format?: (value: number) => string;
  disabled?: boolean;
  logarithmic?: boolean;
};

function toNorm(value: number, min: number, max: number, log: boolean) {
  if (log) {
    const a = Math.log(Math.max(min, 0.0001));
    const b = Math.log(Math.max(max, 0.0001));
    return (Math.log(Math.max(value, 0.0001)) - a) / (b - a);
  }
  return (value - min) / (max - min);
}

function fromNorm(t: number, min: number, max: number, log: boolean) {
  const clamped = Math.min(1, Math.max(0, t));
  if (log) {
    const a = Math.log(Math.max(min, 0.0001));
    const b = Math.log(Math.max(max, 0.0001));
    return Math.exp(a + clamped * (b - a));
  }
  return min + clamped * (max - min);
}

function snap(value: number, step: number) {
  if (step <= 0) return value;
  return Math.round(value / step) * step;
}

export function Knob({
  label,
  value,
  min,
  max,
  step = 0,
  onChange,
  format,
  disabled,
  logarithmic = false,
}: KnobProps) {
  const id = useId();
  const drag = useRef<{ y: number; t: number } | null>(null);
  const numeric = typeof value === "number" && Number.isFinite(value) ? value : min;
  const t = toNorm(numeric, min, max, logarithmic);
  const angle = -135 + t * 270;
  const display = format ? format(numeric) : numeric.toFixed(2);

  const commit = useCallback(
    (nextT: number) => {
      let next = fromNorm(nextT, min, max, logarithmic);
      if (step > 0) next = snap(next, step);
      next = Math.min(max, Math.max(min, next));
      onChange(next);
    },
    [logarithmic, max, min, onChange, step],
  );

  const onPointerDown = (event: React.PointerEvent<HTMLButtonElement>) => {
    if (disabled) return;
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      /* synthetic or already-released pointer */
    }
    drag.current = { y: event.clientY, t };
  };

  const onPointerMove = (event: React.PointerEvent<HTMLButtonElement>) => {
    if (!drag.current) return;
    const dy = drag.current.y - event.clientY;
    commit(drag.current.t + dy / 140);
  };

  const onPointerUp = () => {
    drag.current = null;
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return;
    const dir =
      event.key === "ArrowUp" || event.key === "ArrowRight"
        ? 1
        : event.key === "ArrowDown" || event.key === "ArrowLeft"
          ? -1
          : 0;
    if (!dir) return;
    event.preventDefault();
    const delta = event.shiftKey ? 0.08 : 0.025;
    commit(t + dir * delta);
  };

  return (
    <div className="flex w-16 flex-col items-center gap-1.5 sm:w-[4.5rem]">
      <label
        htmlFor={id}
        className="text-[0.65rem] font-medium uppercase tracking-[0.14em] text-subtle"
      >
        {label}
      </label>
      <button
        id={id}
        type="button"
        disabled={disabled}
        aria-label={`${label} ${display}`}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={Number(numeric.toFixed(3))}
        role="slider"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onKeyDown={onKeyDown}
        className={cn(
          "relative size-14 rounded-full bg-elevated shadow-[var(--shadow-panel)] sm:size-16",
          "touch-none select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
          disabled && "opacity-40",
        )}
      >
        <svg viewBox="0 0 64 64" className="size-full text-subtle" aria-hidden>
          <circle
            cx="32"
            cy="32"
            r="26"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.25"
            opacity="0.35"
          />
          <circle
            cx="32"
            cy="32"
            r="26"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.75"
            strokeDasharray={`${t * 122.5} 122.5`}
            strokeLinecap="round"
            transform="rotate(135 32 32)"
            className="text-accent"
          />
          <line
            x1="32"
            y1="32"
            x2="32"
            y2="12"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            className="text-fg"
            transform={`rotate(${angle} 32 32)`}
          />
        </svg>
      </button>
      <span className="font-mono text-[0.65rem] tabular-nums text-muted">
        {display}
      </span>
    </div>
  );
}
