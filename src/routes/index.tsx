import { createFileRoute } from "@tanstack/react-router";
import { HelixApp } from "@/components/synth/helix-app";

export const Route = createFileRoute("/")({ component: Home });

function Home() {
  return <HelixApp />;
}
