# Helix desktop

Native Linux port of the [Helix](https://github.com/peterhbj/synthemulator) analog keyboard.

- Subtractive synth matching the web engine (saw/square/sine/tri, lowpass, ADSR, compressor)
- Arpeggiator + Map of the Problematique whammy
- QWERTY keyboard, pitch bend, sustain
- MIDI USB input (notes, CC64 sustain, pitch wheel, all-notes-off)
- GT-100 MIDI out: guitar whammy runs on the pedal's P.SHIFT, not a software vocoder

## Install as a desktop app

Puts a **Helix** entry in the app launcher (no need to `cd` or run cargo):

```sh
cd synth-emulator-desktop
./packaging/install.sh
```

That copies the binary to `~/.local/bin/helix-synth` and a `.desktop` file to
`~/.local/share/applications`. Search for “Helix” in the menu, or run
`helix-synth`.

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

## Controls

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
