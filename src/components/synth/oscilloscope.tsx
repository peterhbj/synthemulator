import { useEffect, useRef } from "react";
import { getEngine } from "@/lib/synth/engine";
import { useSynth } from "@/lib/synth/store";

export function Oscilloscope() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const meterRef = useRef<HTMLDivElement>(null);
  const audioReady = useSynth((s) => s.audioReady);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx2d = canvas.getContext("2d");
    if (!ctx2d) return;

    let frame = 0;
    let peak = 0;
    const timeData = new Uint8Array(2048);

    const draw = () => {
      const { width, height } = canvas;
      const engine = getEngine();
      ctx2d.clearRect(0, 0, width, height);
      ctx2d.fillStyle = getComputedStyle(canvas).getPropertyValue("--color-elevated") || "#19191d";
      ctx2d.fillRect(0, 0, width, height);

      ctx2d.strokeStyle = "rgba(243,243,241,0.06)";
      ctx2d.lineWidth = 1;
      ctx2d.beginPath();
      ctx2d.moveTo(0, height / 2);
      ctx2d.lineTo(width, height / 2);
      ctx2d.stroke();

      if (engine && audioReady) {
        engine.analyser.getByteTimeDomainData(timeData);
        ctx2d.strokeStyle = getComputedStyle(canvas).getPropertyValue("--color-glow") || "#d7dee6";
        ctx2d.lineWidth = 1.5;
        ctx2d.beginPath();
        const slice = width / timeData.length;
        let sum = 0;
        for (let i = 0; i < timeData.length; i++) {
          const v = timeData[i] / 128 - 1;
          sum += v * v;
          const x = i * slice;
          const y = (0.5 - v * 0.42) * height;
          if (i === 0) ctx2d.moveTo(x, y);
          else ctx2d.lineTo(x, y);
        }
        ctx2d.stroke();
        const rms = Math.sqrt(sum / timeData.length);
        peak = Math.max(rms, peak * 0.92);
        if (meterRef.current) {
          const pct = Math.min(100, peak * 220);
          meterRef.current.style.height = `${pct}%`;
        }
      } else if (meterRef.current) {
        meterRef.current.style.height = "0%";
      }

      frame = requestAnimationFrame(draw);
    };

    const resize = () => {
      const rect = canvas.getBoundingClientRect();
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      canvas.width = Math.max(1, Math.floor(rect.width * dpr));
      canvas.height = Math.max(1, Math.floor(rect.height * dpr));
    };

    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    frame = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [audioReady]);

  return (
    <div className="flex h-36 min-h-36 gap-2 sm:h-full sm:min-h-40">
      <div className="relative min-w-0 flex-1 overflow-hidden rounded-xl bg-elevated shadow-[var(--shadow-panel)]">
        <canvas ref={canvasRef} className="absolute inset-0 size-full" />
        {!audioReady && (
          <div className="absolute inset-0 grid place-items-center text-[0.7rem] uppercase tracking-[0.16em] text-subtle">
            Scope
          </div>
        )}
      </div>
      <div
        className="relative w-2.5 overflow-hidden rounded-full bg-elevated shadow-[var(--shadow-panel)]"
        aria-hidden
      >
        <div
          ref={meterRef}
          className="absolute inset-x-0 bottom-0 bg-accent transition-[height] duration-75 ease-linear"
          style={{ height: "0%" }}
        />
      </div>
    </div>
  );
}
