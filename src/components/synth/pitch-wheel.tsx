import { useEffect, useRef } from "react";
import { BEND_RANGES, useSynth, type BendRange } from "@/lib/synth/store";
import { cn } from "@/lib/utils";

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function isOtherSlider(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false;
  return target.getAttribute("role") === "slider" && target.dataset.bend !== "true";
}

function isTypingTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    target.isContentEditable
  );
}

export function PitchWheel() {
  const bend = useSynth((s) => s.bend);
  const bendRange = useSynth((s) => s.bendRange);
  const audioReady = useSynth((s) => s.audioReady);
  const setBend = useSynth((s) => s.setBend);
  const setBendRange = useSynth((s) => s.setBendRange);

  const trackRef = useRef<HTMLButtonElement>(null);
  const dragging = useRef(false);
  const springing = useRef(0);
  const arrows = useRef({ up: false, down: false });

  const applyFromClientY = (clientY: number) => {
    const el = trackRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const mid = rect.top + rect.height / 2;
    const t = (mid - clientY) / (rect.height / 2 - 8);
    setBend(clamp(t, -1, 1));
  };

  const springHome = () => {
    if (dragging.current || arrows.current.up || arrows.current.down) return;
    const start = useSynth.getState().bend;
    if (Math.abs(start) < 0.001) {
      setBend(0);
      return;
    }
    const token = ++springing.current;
    const from = start;
    const t0 = performance.now();
    const dur = 180;
    const tick = (now: number) => {
      if (token !== springing.current || dragging.current) return;
      const t = Math.min(1, (now - t0) / dur);
      const eased = 1 - (1 - t) ** 3;
      setBend(from * (1 - eased));
      if (t < 1) requestAnimationFrame(tick);
      else setBend(0);
    };
    requestAnimationFrame(tick);
  };

  const onPointerDown = (event: React.PointerEvent<HTMLButtonElement>) => {
    if (!audioReady) return;
    event.preventDefault();
    dragging.current = true;
    springing.current += 1;
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      /* synthetic pointer */
    }
    applyFromClientY(event.clientY);
  };

  const onPointerMove = (event: React.PointerEvent<HTMLButtonElement>) => {
    if (!dragging.current) return;
    applyFromClientY(event.clientY);
  };

  const onPointerUp = () => {
    if (!dragging.current) return;
    dragging.current = false;
    springHome();
  };

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (isTypingTarget(event.target) || isOtherSlider(event.target)) return;
      if (event.metaKey || event.ctrlKey || event.altKey) return;

      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        if (!useSynth.getState().audioReady) return;
        springing.current += 1;
        if (event.key === "ArrowUp") arrows.current.up = true;
        else arrows.current.down = true;
        const dir = arrows.current.up && !arrows.current.down
          ? 1
          : arrows.current.down && !arrows.current.up
            ? -1
            : 0;
        setBend(dir);
      }
    };

    const up = (event: KeyboardEvent) => {
      if (event.key === "ArrowUp") arrows.current.up = false;
      if (event.key === "ArrowDown") arrows.current.down = false;
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        const dir = arrows.current.up && !arrows.current.down
          ? 1
          : arrows.current.down && !arrows.current.up
            ? -1
            : 0;
        if (dir === 0) springHome();
        else setBend(dir);
      }
    };

    const blur = () => {
      arrows.current.up = false;
      arrows.current.down = false;
      dragging.current = false;
      setBend(0);
    };

    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, [setBend]);

  const semitones = bend * bendRange;
  const label =
    Math.abs(semitones) < 0.05
      ? "0.0"
      : `${semitones > 0 ? "+" : ""}${semitones.toFixed(1)}`;

  return (
    <div className="flex h-44 w-16 shrink-0 flex-col items-center gap-1.5 sm:h-52 sm:w-[4.5rem]">
      <span className="text-2xs font-medium uppercase tracking-[0.14em] text-subtle">
        Bend
      </span>
      <button
        ref={trackRef}
        type="button"
        data-bend="true"
        role="slider"
        aria-label="Pitch bend"
        aria-orientation="vertical"
        aria-valuemin={-1}
        aria-valuemax={1}
        aria-valuenow={Number(bend.toFixed(3))}
        aria-valuetext={`${label} semitones`}
        disabled={!audioReady}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onLostPointerCapture={onPointerUp}
        className={cn(
          "relative min-h-0 w-10 flex-1 touch-none overflow-hidden rounded-full bg-elevated shadow-[var(--shadow-panel)] sm:w-11",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
          audioReady ? "cursor-ns-resize" : "opacity-40",
        )}
      >
        <span
          aria-hidden
          className="pointer-events-none absolute inset-x-1 top-1/2 h-px bg-subtle/50"
        />
        <span
          aria-hidden
          className="pitch-grooves pointer-events-none absolute inset-x-0 h-16 opacity-70"
          style={{
            top: `calc(50% - 2rem + ${-bend * 28}px)`,
          }}
        />
        <span
          aria-hidden
          className={cn(
            "pointer-events-none absolute left-1/2 size-7 -translate-x-1/2 -translate-y-1/2 rounded-full bg-fg shadow-[var(--shadow-panel)] transition-[background-color] duration-100",
            Math.abs(bend) > 0.04 && "bg-accent",
          )}
          style={{ top: `${50 - bend * 38}%` }}
        />
      </button>
      <span className="font-mono text-2xs tabular-nums text-muted">{label}</span>
      <div
        role="radiogroup"
        aria-label="Bend range"
        className="grid w-full grid-cols-3 gap-0.5 rounded-lg bg-elevated p-0.5 shadow-[var(--shadow-panel)]"
      >
        {BEND_RANGES.map((range) => {
          const active = range === bendRange;
          return (
            <button
              key={range}
              type="button"
              role="radio"
              aria-checked={active}
              disabled={!audioReady}
              onClick={() => setBendRange(range as BendRange)}
              className={cn(
                "h-6 rounded-md font-mono text-2xs tabular-nums transition-[background-color,color] duration-150",
                active ? "bg-accent text-accent-fg" : "text-muted hover:text-fg",
              )}
            >
              {range}
            </button>
          );
        })}
      </div>
    </div>
  );
}
