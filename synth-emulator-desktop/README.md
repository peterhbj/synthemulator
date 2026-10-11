# Helix desktop

Native desktop port of the [Helix](https://github.com/peterhbj/synthemulator) analog keyboard,
plus a two-deck live remix tab. Runs on Linux and Windows 10/11.

Two tabs: **Synth** (F1) and **Remix** (F2). F10 turns the touchpad into an XY pad
on either tab.

- Subtractive synth matching the web engine (saw/square/sine/tri, lowpass, ADSR, compressor)
- Arpeggiator + Map of the Problematique whammy
- QWERTY keyboard, pitch bend, sustain
- MIDI USB input (notes, CC64 sustain, pitch wheel, all-notes-off)
- GT-100 MIDI out: guitar whammy runs on the pedal's P.SHIFT, not a software vocoder

## Install as a desktop app (Linux)

Puts a **Helix** entry in the app launcher (no need to `cd` or run cargo):

```sh
cd synth-emulator-desktop
./packaging/install.sh
```

That copies the binary to `~/.local/bin/helix-synth` and a `.desktop` file to
`~/.local/share/applications`. Search for “Helix” in the menu, or run
`helix-synth`.

## Windows

Every push builds `helix.exe` in GitHub Actions (workflow **desktop**, artifact
`helix-Windows`). Download it, put it anywhere and run it: no installer, no
runtime to install. It is unsigned, so SmartScreen asks once (More info, Run
anyway). Settings live in `%APPDATA%\Helix\settings.json`.

## Build from source

Needs Rust, ALSA headers, and a working PipeWire/Pulse/ALSA output.

```sh
# Arch
sudo pacman -S rust alsa-lib pkgconf

cd synth-emulator-desktop
cargo run --release
```

The binary is `target/release/helix`. If `/` is tight on disk, build with `CARGO_TARGET_DIR=/tmp/helix-target cargo build --release`. Then `./packaging/install.sh`.

IBM Plex Sans/Mono (SIL Open Font License) are bundled in `assets/fonts/`.

## Synth tab controls

| Input | Action |
|-------|--------|
| Z row / Q row | Notes (same map as the website) |
| Space | Sustain pedal |
| `[` `]` | Octave down / up |
| Arrow up / down | Pitch bend |
| Esc | Panic (all notes off) |
| MIDI notes | Play, velocity scales gain |
| MIDI CC 64 | Sustain |
| MIDI CC 80 | Whammy tap toggle (GT-100 CTL1). Release is ignored. |
| MIDI CC 81 | Fuzz Factory tap toggle (GT-100 CTL2). Release is ignored. |
| MIDI CC 16–20 | Fuzz Gate / Comp / Stab / Drive / Vol |
| MIDI pitch bend | Bend (range from the 2/7/12 buttons) |

Plug a USB MIDI keyboard (Yamaha, etc.). Helix auto-selects the hardware port
(not Midi Through). The MIDI chip in the header lights when notes arrive.
Refresh if you plugged in after launch; choose **None** to stop auto-connect.

## Remix tab

Two decks (A left, B right) play local files: mp3, flac, wav, ogg, m4a/aac,
alac, aiff. Spotify and other streaming services are DRM-protected and cannot
be loaded. Pick a music folder in the browser on the left; each track is
decoded into memory and analysed for tempo and a beat grid when it loads.
Hot cues and a corrected tempo are remembered per file.

Per deck: three-band isolator EQ with kills, one-knob filter (left low-pass,
right high-pass), beat loops, hot cues, tempo fader (±8/16/50 %), nudge and
sync. Master: crossfader, echo locked to the master deck's tempo, master filter
and a limiter. **Space** runs an automatic transition: the incoming deck syncs,
starts, swaps the bass at the midpoint and the crossfader moves across in the
chosen number of beats.

Keys read the physical position, so they work the same on US and ABNT2 layouts.

| Deck A | Deck B | Action |
|--------|--------|--------|
| `Q` | `P` | Play / pause (Shift: load the selected track) |
| `W` | `O` | Cue (playing: back to cue and stop; stopped: set cue) |
| `1` `2` `3` `4` | `7` `8` `9` `0` | Hot cues (empty: set; set: jump and play; Shift: clear) |
| `A` `S` `D` `F` | `H` `J` `K` `L` | Beat loop 1 / 2 / 4 / 8 (same key again exits) |
| `G` | `;` (`Ç` on ABNT2) | Exit loop |
| `Z` `X` (hold) | `N` `M` (hold) | Nudge slower / faster |
| `C` | `,` | Sync to the other deck |
| `E` `R` (hold) | `U` `I` (hold) | Sweep filter low-pass / high-pass (both keys: reset) |

| Key | Action |
|-----|--------|
| `←` `↓` `→` | Glide the crossfader to A / centre / B |
| Space | Automatic transition to the other deck (again: cancel) |
| Shift+Esc | Pause both decks |

## XY pad (F10)

F10 locks the touchpad and turns it into an XY pad; F10 or Esc gives it back.
Each tab has its own two targets (X and Y), chosen in the row under the tabs:
filters, EQ, crossfader, echo amount/repeats/time on Remix; cutoff, resonance,
pitch bend on Synth. With **volta ao soltar** on, lifting the finger puts the
parameters back where they were. Leaving the window also releases the pad.

- **Linux:** reads the touchpad directly (evdev) and grabs it, so the cursor
  stays still. Your user must be able to read `/dev/input/event*`:
  `sudo usermod -aG input $USER`, then log out and back in. Without that,
  Helix falls back to relative mode (the cursor is hidden and confined, and
  finger movement nudges the puck), and the status line says why.
- **Windows:** reads the Precision Touchpad through raw input and pins the
  cursor while active. Clicks, tap-to-click and system gestures still go
  through; turn off three/four-finger gestures if they get in the way. Without
  a Precision Touchpad it uses relative mode.

## Guitar / GT-100

The Boss GT-100 is both **USB audio** (monitor) and **USB MIDI** (control).

Guitar → GT-100 → USB into Helix for monitoring. Turn **Guitar** on (auto-picks
the GT-100 input). Turn **Whammy** on and Helix sends Roland SysEx to the
temporary patch: FX1 becomes P.SHIFT FAST and the Map of the Problematique
octaves (−12 / 0 / +12 on 16ths) are written to `PS1:PITCH`. The pedal's DSP
transposes the chord; Helix does not pitch-shift the USB audio.

That takeover is temporary (not saved to a USER slot). P.SHIFT runs in **MEDIUM**
(chords stay in tune; FAST warbles individual notes). USB dry-mix is muted while
the whammy is on so the PC doesn’t hear the unshifted guitar on top of the shift,
then restored. Turning Whammy or Guitar off restores the previous FX1 settings.
MIDI out auto-picks **GT-100 MIDI 1**. CTL pedals on that port: **CC#80**
toggles Whammy, **CC#81** toggles Fuzz Factory (value ≥64 = on).
Do not use MIDI 2 as a keyboard — that port is guitar-to-MIDI and will misfire
on chords.

Gain sits under Guitar. Use the Input / GT-100 dropdowns if several USB devices
are present.

No sign-in. Audio starts with the window.

## Layout

Code lives in `synth-emulator-desktop/` so the web app in `src/` stays the spec for sound and controls.

- `src/`: the app (egui UI, synth engine, audio host that mixes the synth over the remix bus)
- `remix/`: decoding, tempo analysis, decks, DSP and the remix mixer (no UI)
- `xypad/`: touchpad backends (Linux evdev, Windows raw input) and the relative fallback
