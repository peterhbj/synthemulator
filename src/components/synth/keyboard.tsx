import { useCallback, useEffect, useRef } from "react";
import {
  COMPUTER_KEY_MAP,
  OFFSET_KEY_LABEL,
  countWhiteKeys,
  isBlackKey,
  midiToName,
  octaveBaseMidi,
  visibleMidis,
  whiteKeyIndex,
} from "@/lib/synth/notes";
import { useSynth } from "@/lib/synth/store";
import { cn } from "@/lib/utils";

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

export function Keyboard() {
  const octave = useSynth((s) => s.octave);
  const activeNotes = useSynth((s) => s.activeNotes);
  const audioReady = useSynth((s) => s.audioReady);
  const noteOn = useSynth((s) => s.noteOn);
  const noteOff = useSynth((s) => s.noteOff);
  const setPedal = useSynth((s) => s.setPedal);
  const shiftOctave = useSynth((s) => s.shiftOctave);
  const panic = useSynth((s) => s.panic);
  const enableAudio = useSynth((s) => s.enableAudio);

  const pointerNotes = useRef(new Map<number, number>());
  const keyHeld = useRef(new Map<string, number>());
  const midis = visibleMidis(octave);
  const whiteCount = countWhiteKeys(midis);
  const base = octaveBaseMidi(octave);
  const active = new Set(activeNotes);

  const press = useCallback(
    (midi: number) => {
      noteOn(midi);
    },
    [noteOn],
  );

  const release = useCallback(
    (midi: number) => {
      noteOff(midi);
    },
    [noteOff],
  );

  const onPointerDown = (midi: number, event: React.PointerEvent) => {
    if (!audioReady) return;
    event.preventDefault();
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {
      /* synthetic or already-released pointer */
    }
    pointerNotes.current.set(event.pointerId, midi);
    press(midi);
  };

  const onPointerUp = (event: React.PointerEvent) => {
    const midi = pointerNotes.current.get(event.pointerId);
    if (midi === undefined) return;
    pointerNotes.current.delete(event.pointerId);
    release(midi);
  };

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (isTypingTarget(event.target) || event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }
      if (event.repeat) return;

      if (event.code === "Space") {
        event.preventDefault();
        if (!useSynth.getState().audioReady) void enableAudio();
        setPedal(true);
        return;
      }
      if (event.key === "[") {
        event.preventDefault();
        shiftOctave(-1);
        return;
      }
      if (event.key === "]") {
        event.preventDefault();
        shiftOctave(1);
        return;
      }
      if (event.key === "Escape") {
        panic();
        return;
      }

      const key = event.key.toLowerCase();
      const offset = COMPUTER_KEY_MAP[key];
      if (offset === undefined) return;
      event.preventDefault();
      if (keyHeld.current.has(key)) return;

      const play = () => {
        const midi = octaveBaseMidi(useSynth.getState().octave) + offset;
        keyHeld.current.set(key, midi);
        press(midi);
      };

      if (!useSynth.getState().audioReady) {
        void enableAudio().then(play);
        return;
      }
      play();
    };

    const up = (event: KeyboardEvent) => {
      if (event.code === "Space") {
        event.preventDefault();
        setPedal(false);
        return;
      }
      const key = event.key.toLowerCase();
      const midi = keyHeld.current.get(key);
      if (midi === undefined) return;
      keyHeld.current.delete(key);
      release(midi);
    };

    const blur = () => {
      keyHeld.current.clear();
      pointerNotes.current.clear();
      panic();
    };

    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, [enableAudio, panic, press, release, setPedal, shiftOctave]);

  return (
    <div className="-mx-1 overflow-x-auto px-1 [scrollbar-width:thin]">
      <div
        className="relative h-44 min-w-[40rem] touch-none sm:h-52 sm:min-w-0"
        role="group"
        aria-label="Piano keyboard"
      >
        {midis
          .filter((midi) => !isBlackKey(midi))
          .map((midi) => {
            const lit = active.has(midi);
            const label = OFFSET_KEY_LABEL[midi - base];
            return (
              <button
                key={midi}
                type="button"
                tabIndex={-1}
                aria-label={midiToName(midi)}
                aria-pressed={lit}
                disabled={!audioReady}
                onPointerDown={(e) => onPointerDown(midi, e)}
                onPointerUp={onPointerUp}
                onPointerCancel={onPointerUp}
                onLostPointerCapture={onPointerUp}
                className={cn(
                  "absolute bottom-0 top-0 rounded-b-md bg-key text-key-fg shadow-[inset_0_-10px_18px_color-mix(in_oklab,var(--color-key-fg)_8%,transparent)]",
                  "transition-[background-color,transform,box-shadow] duration-100 ease-out focus-visible:outline-none",
                  lit &&
                    "translate-y-0.5 bg-glow shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-bg)_12%,transparent)]",
                  !audioReady && "opacity-70",
                )}
                style={{
                  left: `${(whiteKeyIndex(midi, base) / whiteCount) * 100}%`,
                  width: `${100 / whiteCount}%`,
                }}
              >
                <span className="absolute inset-x-0 bottom-2 flex flex-col items-center gap-0.5">
                  <span className="hidden text-2xs font-medium uppercase tracking-wider text-key-fg/45 sm:block">
                    {label}
                  </span>
                  <span className="font-mono text-2xs tabular-nums text-key-fg/55">
                    {midiToName(midi)}
                  </span>
                </span>
              </button>
            );
          })}

        {midis
          .filter((midi) => isBlackKey(midi))
          .map((midi) => {
            const lit = active.has(midi);
            const label = OFFSET_KEY_LABEL[midi - base];
            const afterWhite = whiteKeyIndex(midi, base);
            const whiteW = 100 / whiteCount;
            const width = whiteW * 0.62;
            const left = afterWhite * whiteW - width / 2;
            return (
              <button
                key={midi}
                type="button"
                tabIndex={-1}
                aria-label={midiToName(midi)}
                aria-pressed={lit}
                disabled={!audioReady}
                onPointerDown={(e) => onPointerDown(midi, e)}
                onPointerUp={onPointerUp}
                onPointerCancel={onPointerUp}
                onLostPointerCapture={onPointerUp}
                className={cn(
                  "absolute top-0 z-10 rounded-b-md bg-key-sharp text-key-sharp-fg shadow-[var(--shadow-panel)]",
                  "transition-[background-color,transform] duration-100 ease-out focus-visible:outline-none",
                  lit && "translate-y-0.5 bg-fg text-accent-fg",
                )}
                style={{
                  left: `${left}%`,
                  width: `${width}%`,
                  height: "58%",
                }}
              >
                <span className="absolute inset-x-0 bottom-2 hidden text-center text-2xs font-medium tracking-wider text-current/50 sm:block">
                  {label}
                </span>
              </button>
            );
          })}
      </div>
    </div>
  );
}
