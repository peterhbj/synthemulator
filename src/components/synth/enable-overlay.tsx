import { Play } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useSynth } from "@/lib/synth/store";

export function EnableOverlay() {
  const audioReady = useSynth((s) => s.audioReady);
  const enableAudio = useSynth((s) => s.enableAudio);

  if (audioReady) return null;

  return (
    <div className="absolute inset-0 z-20 flex items-center justify-center rounded-[1.25rem] bg-bg/80 p-6">
      <div className="flex max-w-sm flex-col items-center gap-4 text-center">
        <p className="text-sm text-muted">
          Audio waits for a tap or key so the browser can start the engine.
        </p>
        <Button
          type="button"
          size="lg"
          onClick={() => void enableAudio()}
          className="min-w-44"
        >
          <Play className="size-4 translate-x-px" />
          Enable audio
        </Button>
        <p className="text-xs text-subtle">Or press any note key</p>
      </div>
    </div>
  );
}
