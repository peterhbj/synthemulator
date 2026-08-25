# Helix desktop

Native Linux port of the [Helix](https://github.com/peterhbj/synthemulator) analog keyboard.

- Subtractive synth matching the web engine (saw/square/sine/tri, lowpass, ADSR, compressor)
- Arpeggiator + Map of the Problematique whammy
- QWERTY keyboard, pitch bend, sustain
- MIDI USB input (notes, CC64 sustain, pitch wheel, all-notes-off)

## Build

Needs Rust, ALSA headers, and a working PipeWire/Pulse/ALSA output.

```sh
# Arch
sudo pacman -S rust alsa-lib pkgconf

cd synth-emulator-desktop
cargo run --release
```

The binary is `target/release/helix`. If `/` is tight on disk, build with `CARGO_TARGET_DIR=/tmp/helix-target cargo build --release`.

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

No sign-in. Audio starts with the window.

## Layout

Code lives in `synth-emulator-desktop/` so the web app in `src/` stays the spec for sound and controls.
