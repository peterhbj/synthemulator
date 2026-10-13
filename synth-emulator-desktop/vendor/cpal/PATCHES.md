# Helix patches to the vendored cpal

Upstream: cpal 0.16.0 from crates.io. Everything here is Linux ALSA; the rest of the
crate is unchanged apart from the `allow(dead_code)` below.

## 1. Recover the PCM by its state (`src/host/alsa/mod.rs`)

Upstream answers an xrun with a bare `snd_pcm_prepare`. That fails on a suspended or
unplugged device, and then the worker loops at full CPU, reporting an error each time.

The patch adds `recover_pcm`, which asks `recovery_errno` for the errno from the PCM state:

- `XRun`, `Prepared`, `Running` → `EPIPE` (prepare)
- `Suspended` → `ESTRPIPE` (resume)
- `Disconnected` → no recovery; the worker reports `DeviceNotAvailable` once and stops.

Capture streams stay `PREPARED` after recovery and never start on their own, so
`recover_pcm` calls `start()` for inputs. Playback starts at its start threshold.

`POLLERR` also goes through this path (it used to be an error that the worker retried
forever), so a PipeWire xrun is handled as an xrun.

Tests: `recovery_tests` at the end of `src/host/alsa/mod.rs`.

## 2. Silence an upstream dead-code warning (`src/platform/mod.rs`)

`NotSendSyncAcrossAllPlatforms` is never constructed, which warns on every build.
This is the only change outside the ALSA backend.

## When to drop this

- Drop patch 1 when upstream cpal recovers from xruns and suspends by state, and Helix
  moves to that release. Check `src/host/alsa/mod.rs` in the new version first.
- Drop patch 2 with patch 1, or when cpal stops warning on it.

Helix also depends on this vendored copy for the HDMI fix: 128-frame buffers stop the
callback, so the app asks for 512 frames (see `src/audio/host.rs`).
