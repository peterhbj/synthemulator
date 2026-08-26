# Helix desktop

Native Linux port of the [Helix](https://github.com/peterhbj/synthemulator) analog keyboard.

- Subtractive synth matching the web engine (saw/square/sine/tri, lowpass, ADSR, compressor)
- Arpeggiator + Map of the Problematique whammy
- QWERTY keyboard, pitch bend, sustain
- MIDI USB input (notes, CC64 sustain, pitch wheel, all-notes-off)

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
| MIDI pitch bend | Bend (range from the 2/7/12 buttons) |

Plug a USB MIDI keyboard (Yamaha, etc.). Helix auto-selects the hardware port
(not Midi Through). The MIDI chip in the header lights when notes arrive.
Refresh if you plugged in after launch; choose **None** to stop auto-connect.

## Guitar / GT-100

The Boss GT-100 shows up as **USB audio**, not notes. Guitar → GT-100 input → USB
into Helix. Turn **Guitar** on (it auto-picks `GT-100`), turn **Whammy** on, and
the Map of the Problematique octave jumps (−1 / 0 / +1) run on the guitar through
the same filter as the synth. Gain sits under Guitar. Use the Input dropdown if
several USB devices are present.

No sign-in. Audio starts with the window.

## Layout

Code lives in `synth-emulator-desktop/` so the web app in `src/` stays the spec for sound and controls.
