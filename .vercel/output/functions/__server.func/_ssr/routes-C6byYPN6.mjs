import { o as __toESM } from "../_runtime.mjs";
import { o as require_jsx_runtime, s as require_react } from "../_libs/@radix-ui/react-collection+[...].mjs";
import { a as Minus, i as Play, r as Plus, t as Volume2 } from "../_libs/lucide-react.mjs";
import { a as signOut, n as authClient, r as cn, t as Button } from "./button-BsdhwErW.mjs";
import { t as create } from "../_libs/zustand.mjs";
import { i as SliderTrack, n as SliderRange, r as SliderThumb, t as Slider$1 } from "../_libs/@radix-ui/react-slider+[...].mjs";
//#region node_modules/.nitro/vite/services/ssr/assets/routes-C6byYPN6.js
var import_react = /* @__PURE__ */ __toESM(require_react());
var import_jsx_runtime = require_jsx_runtime();
/**
* Current user + loading state. Same behavior in live preview and when deployed:
*   - Auth enabled (default) -> the real signed-in user; `user` is `null` while
*                            the session resolves (`isPending: true`) and when
*                            signed out (`isPending: false`). Session comes from
*                            Better Auth `useSession()` → `/api/auth/get-session`
*                            (cookie when deployed; bearer in live preview).
*   - Auth disabled (`VITE_AUTH_ENABLED=false`) -> `DEV_USER`, never pending.
*
* Protect a route by waiting out `isPending` before acting on `user` —
* redirecting on `user: null` alone bounces signed-in visitors to sign-in on
* every hard reload:
*
*   import { RedirectToSignIn } from "@/lib/auth/gates";
*   const { user, isPending } = useCurrentUserState();
*   if (isPending) return null;              // still resolving — don't redirect yet
*   if (!user) return <RedirectToSignIn />;  // definitely signed out
*
* `authEnabled` is a module-level constant fixed at load, so the guarded hook
* call keeps a stable hook order across every render of a given component.
*/
function useCurrentUserState() {
	const { data, isPending } = authClient.useSession();
	const user = data?.user;
	return {
		user: user ? {
			id: user.id,
			displayName: user.name ?? null,
			primaryEmail: user.email ?? null,
			profileImageUrl: user.image ?? null,
			isDevFallback: false
		} : null,
		isPending
	};
}
/**
* Convenience view of `useCurrentUserState().user` for display (e.g.
* `user?.displayName ?? "Guest"`). NOTE: `null` means *loading OR signed out* —
* for redirects/guards use `useCurrentUserState()` and check `isPending`.
*/
function useCurrentUser() {
	return useCurrentUserState().user;
}
/**
* Minimal signed-in identity chip + sign-out. Restyle freely (see the
* `design-ui` skill). Sign-out is only shown when auth is enabled (the
* disabled-auth dev user has nothing to sign out of).
*/
function UserButton() {
	const user = useCurrentUser();
	if (!user) return null;
	const label = user.displayName ?? user.primaryEmail ?? "Account";
	return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
		className: "flex items-center gap-2",
		children: [
			user.profileImageUrl ? /* @__PURE__ */ (0, import_jsx_runtime.jsx)("img", {
				src: user.profileImageUrl,
				alt: "",
				className: "size-8 rounded-full object-cover outline outline-1 -outline-offset-1 outline-fg/10"
			}) : /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
				className: "grid size-8 place-items-center rounded-full bg-elevated text-sm font-medium text-fg",
				children: label.charAt(0).toUpperCase()
			}),
			/* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
				className: "hidden max-w-28 truncate text-sm font-medium text-fg sm:inline",
				children: label
			}),
			/* @__PURE__ */ (0, import_jsx_runtime.jsx)("button", {
				type: "button",
				onClick: () => void signOut(),
				className: "text-sm text-muted underline-offset-4 transition-colors duration-150 hover:text-fg hover:underline",
				children: "Sign out"
			})
		]
	});
}
function AuthSlot() {
	const { user, isPending } = useCurrentUserState();
	if (isPending) return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "size-8 animate-pulse rounded-full bg-elevated" });
	if (user) return /* @__PURE__ */ (0, import_jsx_runtime.jsx)(UserButton, {});
	return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("a", {
		href: "/login",
		className: "inline-flex h-8 items-center rounded-md px-3 text-sm font-medium text-muted transition-colors duration-150 hover:bg-elevated hover:text-fg",
		children: "Sign in"
	});
}
var NOTE_NAMES = [
	"C",
	"C#",
	"D",
	"D#",
	"E",
	"F",
	"F#",
	"G",
	"G#",
	"A",
	"A#",
	"B"
];
var BLACK_PCS = [
	1,
	3,
	6,
	8,
	10
];
/** Computer-key → semitone offset from the current octave's C. */
var COMPUTER_KEY_MAP = {
	z: 0,
	s: 1,
	x: 2,
	d: 3,
	c: 4,
	v: 5,
	g: 6,
	b: 7,
	h: 8,
	n: 9,
	j: 10,
	m: 11,
	",": 12,
	l: 13,
	".": 14,
	";": 15,
	"/": 16,
	q: 12,
	"2": 13,
	w: 14,
	"3": 15,
	e: 16,
	r: 17,
	"5": 18,
	t: 19,
	"6": 20,
	y: 21,
	"7": 22,
	u: 23,
	i: 24,
	"9": 25,
	o: 26,
	"0": 27,
	p: 28
};
/** Inverse: midi offset → preferred computer-key label for the on-screen keys. */
var OFFSET_KEY_LABEL = {
	0: "Z",
	1: "S",
	2: "X",
	3: "D",
	4: "C",
	5: "V",
	6: "G",
	7: "B",
	8: "H",
	9: "N",
	10: "J",
	11: "M",
	12: "Q",
	13: "2",
	14: "W",
	15: "3",
	16: "E",
	17: "R",
	18: "5",
	19: "T",
	20: "6",
	21: "Y",
	22: "7",
	23: "U",
	24: "I"
};
function midiToHz(midi) {
	return 440 * 2 ** ((midi - 69) / 12);
}
function midiToName(midi) {
	const pc = (midi % 12 + 12) % 12;
	const octave = Math.floor(midi / 12) - 1;
	return `${NOTE_NAMES[pc]}${octave}`;
}
function isBlackKey(midi) {
	const pc = (midi % 12 + 12) % 12;
	return BLACK_PCS.includes(pc);
}
function octaveBaseMidi(octave) {
	return (octave + 1) * 12;
}
function visibleMidis(octave) {
	const base = octaveBaseMidi(octave);
	return Array.from({ length: 25 }, (_, i) => base + i);
}
function whiteKeyIndex(midi, baseMidi) {
	let index = 0;
	for (let m = baseMidi; m < midi; m++) if (!isBlackKey(m)) index += 1;
	return index;
}
function countWhiteKeys(midis) {
	return midis.filter((m) => !isBlackKey(m)).length;
}
var DEFAULT_PARAMS = {
	waveform: "sawtooth",
	cutoff: 2200,
	resonance: 1.8,
	attack: .018,
	decay: .16,
	sustain: .62,
	release: .28,
	volume: .72
};
var MAX_VOICES = 12;
var VOICE_GAIN = .16;
var MIN_ENV = .004;
var SynthEngine = class {
	ctx;
	analyser;
	filter;
	master;
	voices = /* @__PURE__ */ new Map();
	params = { ...DEFAULT_PARAMS };
	constructor() {
		const ctx = new AudioContext();
		const filter = ctx.createBiquadFilter();
		filter.type = "lowpass";
		filter.frequency.value = this.params.cutoff;
		filter.Q.value = this.params.resonance;
		const compressor = ctx.createDynamicsCompressor();
		compressor.threshold.value = -14;
		compressor.knee.value = 10;
		compressor.ratio.value = 3.5;
		compressor.attack.value = .004;
		compressor.release.value = .12;
		const master = ctx.createGain();
		master.gain.value = this.params.volume;
		const analyser = ctx.createAnalyser();
		analyser.fftSize = 2048;
		analyser.smoothingTimeConstant = .35;
		filter.connect(compressor);
		compressor.connect(master);
		master.connect(analyser);
		analyser.connect(ctx.destination);
		this.ctx = ctx;
		this.filter = filter;
		this.master = master;
		this.analyser = analyser;
	}
	async resume() {
		if (this.ctx.state === "suspended") await this.ctx.resume();
	}
	setParams(next) {
		this.params = {
			...this.params,
			...next
		};
		const now = this.ctx.currentTime;
		if (next.cutoff !== void 0) this.filter.frequency.setTargetAtTime(next.cutoff, now, .03);
		if (next.resonance !== void 0) this.filter.Q.setTargetAtTime(next.resonance, now, .03);
		if (next.volume !== void 0) this.master.gain.setTargetAtTime(next.volume, now, .02);
		if (next.waveform !== void 0) {
			for (const voice of this.voices.values()) if (!voice.stopping) voice.osc.type = next.waveform;
		}
	}
	noteOn(midi) {
		if (this.ctx.state !== "running") return;
		const existing = this.voices.get(midi);
		if (existing) {
			this.releaseVoice(existing, .012);
			this.voices.delete(midi);
		}
		if (this.voices.size >= MAX_VOICES) this.stealOldest();
		const now = this.ctx.currentTime;
		const { attack, decay, sustain, waveform } = this.params;
		const atk = Math.max(MIN_ENV, attack);
		const dec = Math.max(MIN_ENV, decay);
		const peak = VOICE_GAIN;
		const sus = Math.max(1e-4, peak * sustain);
		const osc = this.ctx.createOscillator();
		osc.type = waveform;
		osc.frequency.setValueAtTime(midiToHz(midi), now);
		const gain = this.ctx.createGain();
		gain.gain.setValueAtTime(1e-4, now);
		gain.gain.exponentialRampToValueAtTime(peak, now + atk);
		gain.gain.exponentialRampToValueAtTime(sus, now + atk + dec);
		osc.connect(gain);
		gain.connect(this.filter);
		osc.start(now);
		this.voices.set(midi, {
			midi,
			osc,
			gain,
			startedAt: now,
			stopping: false
		});
	}
	noteOff(midi, releaseOverride) {
		const voice = this.voices.get(midi);
		if (!voice || voice.stopping) return;
		this.releaseVoice(voice, releaseOverride ?? this.params.release);
	}
	allNotesOff() {
		for (const midi of [...this.voices.keys()]) this.noteOff(midi, .04);
	}
	dispose() {
		this.allNotesOff();
		this.ctx.close();
	}
	stealOldest() {
		let oldest;
		for (const voice of this.voices.values()) if (!oldest || voice.startedAt < oldest.startedAt) oldest = voice;
		if (oldest) this.noteOff(oldest.midi, .02);
	}
	releaseVoice(voice, releaseTime) {
		if (voice.stopping) return;
		voice.stopping = true;
		const now = this.ctx.currentTime;
		const release = Math.max(MIN_ENV, releaseTime);
		const current = Math.max(voice.gain.gain.value, 1e-4);
		voice.gain.gain.cancelScheduledValues(now);
		voice.gain.gain.setValueAtTime(current, now);
		voice.gain.gain.exponentialRampToValueAtTime(1e-4, now + release);
		try {
			voice.osc.stop(now + release + .03);
		} catch {}
		window.setTimeout(() => {
			try {
				voice.osc.disconnect();
				voice.gain.disconnect();
			} catch {}
			if (this.voices.get(voice.midi) === voice) this.voices.delete(voice.midi);
		}, (release + .05) * 1e3);
	}
};
var engine = null;
function getEngine() {
	return engine;
}
async function enableEngine() {
	if (!engine) engine = new SynthEngine();
	await engine.resume();
	return engine;
}
function pushParams(partial) {
	getEngine()?.setParams(partial);
}
var useSynth = create((set, get) => ({
	...DEFAULT_PARAMS,
	octave: 3,
	audioReady: false,
	activeNotes: [],
	heldNotes: [],
	pedal: false,
	setWaveform: (waveform) => {
		pushParams({ waveform });
		set({ waveform });
	},
	setCutoff: (cutoff) => {
		pushParams({ cutoff });
		set({ cutoff });
	},
	setResonance: (resonance) => {
		pushParams({ resonance });
		set({ resonance });
	},
	setAttack: (attack) => {
		pushParams({ attack });
		set({ attack });
	},
	setDecay: (decay) => {
		pushParams({ decay });
		set({ decay });
	},
	setSustainLevel: (sustain) => {
		pushParams({ sustain });
		set({ sustain });
	},
	setRelease: (release) => {
		pushParams({ release });
		set({ release });
	},
	setVolume: (volume) => {
		pushParams({ volume });
		set({ volume });
	},
	shiftOctave: (delta) => {
		set((s) => ({ octave: Math.min(6, Math.max(1, s.octave + delta)) }));
	},
	enableAudio: async () => {
		const engine = await enableEngine();
		const { waveform, cutoff, resonance, attack, decay, sustain, release, volume } = get();
		engine.setParams({
			waveform,
			cutoff,
			resonance,
			attack,
			decay,
			sustain,
			release,
			volume
		});
		set({ audioReady: true });
	},
	noteOn: (midi) => {
		const engine = getEngine();
		if (!engine) return;
		engine.noteOn(midi);
		set((s) => ({
			activeNotes: s.activeNotes.includes(midi) ? s.activeNotes : [...s.activeNotes, midi],
			heldNotes: s.heldNotes.includes(midi) ? s.heldNotes : [...s.heldNotes, midi]
		}));
	},
	noteOff: (midi) => {
		const { pedal } = get();
		set((s) => ({ heldNotes: s.heldNotes.filter((n) => n !== midi) }));
		if (pedal) return;
		getEngine()?.noteOff(midi);
		set((s) => ({ activeNotes: s.activeNotes.filter((n) => n !== midi) }));
	},
	setPedal: (on) => {
		if (on) {
			set({ pedal: true });
			return;
		}
		const { heldNotes, activeNotes } = get();
		const held = new Set(heldNotes);
		const releasing = activeNotes.filter((n) => !held.has(n));
		for (const midi of releasing) getEngine()?.noteOff(midi);
		set({
			pedal: false,
			activeNotes: activeNotes.filter((n) => held.has(n))
		});
	},
	panic: () => {
		getEngine()?.allNotesOff();
		set({
			activeNotes: [],
			heldNotes: [],
			pedal: false
		});
	}
}));
function EnableOverlay() {
	const audioReady = useSynth((s) => s.audioReady);
	const enableAudio = useSynth((s) => s.enableAudio);
	if (audioReady) return null;
	return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", {
		className: "absolute inset-0 z-20 flex items-center justify-center rounded-[1.25rem] bg-bg/80 p-6",
		children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
			className: "flex max-w-sm flex-col items-center gap-4 text-center",
			children: [
				/* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", {
					className: "text-sm text-muted",
					children: "Audio waits for a tap or key so the browser can start the engine."
				}),
				/* @__PURE__ */ (0, import_jsx_runtime.jsxs)(Button, {
					type: "button",
					size: "lg",
					onClick: () => void enableAudio(),
					className: "min-w-44",
					children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Play, { className: "size-4 translate-x-px" }), "Enable audio"]
				}),
				/* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", {
					className: "text-xs text-subtle",
					children: "Or press any note key"
				})
			]
		})
	});
}
function isTypingTarget(target) {
	if (!(target instanceof HTMLElement)) return false;
	const tag = target.tagName;
	return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}
function Keyboard() {
	const octave = useSynth((s) => s.octave);
	const activeNotes = useSynth((s) => s.activeNotes);
	const audioReady = useSynth((s) => s.audioReady);
	const noteOn = useSynth((s) => s.noteOn);
	const noteOff = useSynth((s) => s.noteOff);
	const setPedal = useSynth((s) => s.setPedal);
	const shiftOctave = useSynth((s) => s.shiftOctave);
	const panic = useSynth((s) => s.panic);
	const enableAudio = useSynth((s) => s.enableAudio);
	const pointerNotes = (0, import_react.useRef)(/* @__PURE__ */ new Map());
	const keyHeld = (0, import_react.useRef)(/* @__PURE__ */ new Map());
	const midis = visibleMidis(octave);
	const whiteCount = countWhiteKeys(midis);
	const base = octaveBaseMidi(octave);
	const active = new Set(activeNotes);
	const press = (0, import_react.useCallback)((midi) => {
		noteOn(midi);
	}, [noteOn]);
	const release = (0, import_react.useCallback)((midi) => {
		noteOff(midi);
	}, [noteOff]);
	const onPointerDown = (midi, event) => {
		if (!audioReady) return;
		event.preventDefault();
		try {
			event.currentTarget.setPointerCapture(event.pointerId);
		} catch {}
		pointerNotes.current.set(event.pointerId, midi);
		press(midi);
	};
	const onPointerUp = (event) => {
		const midi = pointerNotes.current.get(event.pointerId);
		if (midi === void 0) return;
		pointerNotes.current.delete(event.pointerId);
		release(midi);
	};
	(0, import_react.useEffect)(() => {
		const down = (event) => {
			if (isTypingTarget(event.target) || event.metaKey || event.ctrlKey || event.altKey) return;
			if (event.repeat) return;
			if (event.code === "Space") {
				event.preventDefault();
				if (!useSynth.getState().audioReady) enableAudio();
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
			if (offset === void 0) return;
			event.preventDefault();
			if (keyHeld.current.has(key)) return;
			const play = () => {
				const midi = octaveBaseMidi(useSynth.getState().octave) + offset;
				keyHeld.current.set(key, midi);
				press(midi);
			};
			if (!useSynth.getState().audioReady) {
				enableAudio().then(play);
				return;
			}
			play();
		};
		const up = (event) => {
			if (event.code === "Space") {
				event.preventDefault();
				setPedal(false);
				return;
			}
			const key = event.key.toLowerCase();
			const midi = keyHeld.current.get(key);
			if (midi === void 0) return;
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
	}, [
		enableAudio,
		panic,
		press,
		release,
		setPedal,
		shiftOctave
	]);
	return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", {
		className: "-mx-1 overflow-x-auto px-1 [scrollbar-width:thin]",
		children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
			className: "relative h-44 min-w-[40rem] touch-none sm:h-52 sm:min-w-0",
			role: "group",
			"aria-label": "Piano keyboard",
			children: [midis.filter((midi) => !isBlackKey(midi)).map((midi) => {
				const lit = active.has(midi);
				const label = OFFSET_KEY_LABEL[midi - base];
				return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("button", {
					type: "button",
					tabIndex: -1,
					"aria-label": midiToName(midi),
					"aria-pressed": lit,
					disabled: !audioReady,
					onPointerDown: (e) => onPointerDown(midi, e),
					onPointerUp,
					onPointerCancel: onPointerUp,
					onLostPointerCapture: onPointerUp,
					className: cn("absolute bottom-0 top-0 rounded-b-md bg-key text-key-fg shadow-[inset_0_-10px_18px_color-mix(in_oklab,var(--color-key-fg)_8%,transparent)]", "transition-[background-color,transform,box-shadow] duration-100 ease-out focus-visible:outline-none", lit && "translate-y-0.5 bg-glow shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-bg)_12%,transparent)]", !audioReady && "opacity-70"),
					style: {
						left: `${whiteKeyIndex(midi, base) / whiteCount * 100}%`,
						width: `${100 / whiteCount}%`
					},
					children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("span", {
						className: "absolute inset-x-0 bottom-2 flex flex-col items-center gap-0.5",
						children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
							className: "hidden text-2xs font-medium uppercase tracking-wider text-key-fg/45 sm:block",
							children: label
						}), /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
							className: "font-mono text-2xs tabular-nums text-key-fg/55",
							children: midiToName(midi)
						})]
					})
				}, midi);
			}), midis.filter((midi) => isBlackKey(midi)).map((midi) => {
				const lit = active.has(midi);
				const label = OFFSET_KEY_LABEL[midi - base];
				const afterWhite = whiteKeyIndex(midi, base);
				const whiteW = 100 / whiteCount;
				const width = whiteW * .62;
				const left = afterWhite * whiteW - width / 2;
				return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("button", {
					type: "button",
					tabIndex: -1,
					"aria-label": midiToName(midi),
					"aria-pressed": lit,
					disabled: !audioReady,
					onPointerDown: (e) => onPointerDown(midi, e),
					onPointerUp,
					onPointerCancel: onPointerUp,
					onLostPointerCapture: onPointerUp,
					className: cn("absolute top-0 z-10 rounded-b-md bg-key-sharp text-key-sharp-fg shadow-[var(--shadow-panel)]", "transition-[background-color,transform] duration-100 ease-out focus-visible:outline-none", lit && "translate-y-0.5 bg-fg text-accent-fg"),
					style: {
						left: `${left}%`,
						width: `${width}%`,
						height: "58%"
					},
					children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
						className: "absolute inset-x-0 bottom-2 hidden text-center text-2xs font-medium tracking-wider text-current/50 sm:block",
						children: label
					})
				}, midi);
			})]
		})
	});
}
function toNorm(value, min, max, log) {
	if (log) {
		const a = Math.log(Math.max(min, 1e-4));
		const b = Math.log(Math.max(max, 1e-4));
		return (Math.log(Math.max(value, 1e-4)) - a) / (b - a);
	}
	return (value - min) / (max - min);
}
function fromNorm(t, min, max, log) {
	const clamped = Math.min(1, Math.max(0, t));
	if (log) {
		const a = Math.log(Math.max(min, 1e-4));
		return Math.exp(a + clamped * (Math.log(Math.max(max, 1e-4)) - a));
	}
	return min + clamped * (max - min);
}
function snap(value, step) {
	if (step <= 0) return value;
	return Math.round(value / step) * step;
}
function Knob({ label, value, min, max, step = 0, onChange, format, disabled, logarithmic = false }) {
	const id = (0, import_react.useId)();
	const drag = (0, import_react.useRef)(null);
	const numeric = typeof value === "number" && Number.isFinite(value) ? value : min;
	const t = toNorm(numeric, min, max, logarithmic);
	const angle = -135 + t * 270;
	const display = format ? format(numeric) : numeric.toFixed(2);
	const commit = (0, import_react.useCallback)((nextT) => {
		let next = fromNorm(nextT, min, max, logarithmic);
		if (step > 0) next = snap(next, step);
		next = Math.min(max, Math.max(min, next));
		onChange(next);
	}, [
		logarithmic,
		max,
		min,
		onChange,
		step
	]);
	const onPointerDown = (event) => {
		if (disabled) return;
		try {
			event.currentTarget.setPointerCapture(event.pointerId);
		} catch {}
		drag.current = {
			y: event.clientY,
			t
		};
	};
	const onPointerMove = (event) => {
		if (!drag.current) return;
		const dy = drag.current.y - event.clientY;
		commit(drag.current.t + dy / 140);
	};
	const onPointerUp = () => {
		drag.current = null;
	};
	const onKeyDown = (event) => {
		if (disabled) return;
		const dir = event.key === "ArrowUp" || event.key === "ArrowRight" ? 1 : event.key === "ArrowDown" || event.key === "ArrowLeft" ? -1 : 0;
		if (!dir) return;
		event.preventDefault();
		const delta = event.shiftKey ? .08 : .025;
		commit(t + dir * delta);
	};
	return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
		className: "flex w-16 flex-col items-center gap-1.5 sm:w-[4.5rem]",
		children: [
			/* @__PURE__ */ (0, import_jsx_runtime.jsx)("label", {
				htmlFor: id,
				className: "text-[0.65rem] font-medium uppercase tracking-[0.14em] text-subtle",
				children: label
			}),
			/* @__PURE__ */ (0, import_jsx_runtime.jsx)("button", {
				id,
				type: "button",
				disabled,
				"aria-label": `${label} ${display}`,
				"aria-valuemin": min,
				"aria-valuemax": max,
				"aria-valuenow": Number(numeric.toFixed(3)),
				role: "slider",
				onPointerDown,
				onPointerMove,
				onPointerUp,
				onPointerCancel: onPointerUp,
				onKeyDown,
				className: cn("relative size-14 rounded-full bg-elevated shadow-[var(--shadow-panel)] sm:size-16", "touch-none select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50", disabled && "opacity-40"),
				children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("svg", {
					viewBox: "0 0 64 64",
					className: "size-full text-subtle",
					"aria-hidden": true,
					children: [
						/* @__PURE__ */ (0, import_jsx_runtime.jsx)("circle", {
							cx: "32",
							cy: "32",
							r: "26",
							fill: "none",
							stroke: "currentColor",
							strokeWidth: "1.25",
							opacity: "0.35"
						}),
						/* @__PURE__ */ (0, import_jsx_runtime.jsx)("circle", {
							cx: "32",
							cy: "32",
							r: "26",
							fill: "none",
							stroke: "currentColor",
							strokeWidth: "1.75",
							strokeDasharray: `${t * 122.5} 122.5`,
							strokeLinecap: "round",
							transform: "rotate(135 32 32)",
							className: "text-accent"
						}),
						/* @__PURE__ */ (0, import_jsx_runtime.jsx)("line", {
							x1: "32",
							y1: "32",
							x2: "32",
							y2: "12",
							stroke: "currentColor",
							strokeWidth: "2",
							strokeLinecap: "round",
							className: "text-fg",
							transform: `rotate(${angle} 32 32)`
						})
					]
				})
			}),
			/* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
				className: "font-mono text-[0.65rem] tabular-nums text-muted",
				children: display
			})
		]
	});
}
function Oscilloscope() {
	const canvasRef = (0, import_react.useRef)(null);
	const meterRef = (0, import_react.useRef)(null);
	const audioReady = useSynth((s) => s.audioReady);
	(0, import_react.useEffect)(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;
		const ctx2d = canvas.getContext("2d");
		if (!ctx2d) return;
		let frame = 0;
		let peak = 0;
		const timeData = /* @__PURE__ */ new Uint8Array(2048);
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
					const y = (.5 - v * .42) * height;
					if (i === 0) ctx2d.moveTo(x, y);
					else ctx2d.lineTo(x, y);
				}
				ctx2d.stroke();
				const rms = Math.sqrt(sum / timeData.length);
				peak = Math.max(rms, peak * .92);
				if (meterRef.current) {
					const pct = Math.min(100, peak * 220);
					meterRef.current.style.height = `${pct}%`;
				}
			} else if (meterRef.current) meterRef.current.style.height = "0%";
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
	return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
		className: "flex h-36 min-h-36 gap-2 sm:h-full sm:min-h-40",
		children: [/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
			className: "relative min-w-0 flex-1 overflow-hidden rounded-xl bg-elevated shadow-[var(--shadow-panel)]",
			children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)("canvas", {
				ref: canvasRef,
				className: "absolute inset-0 size-full"
			}), !audioReady && /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", {
				className: "absolute inset-0 grid place-items-center text-[0.7rem] uppercase tracking-[0.16em] text-subtle",
				children: "Scope"
			})]
		}), /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", {
			className: "relative w-2.5 overflow-hidden rounded-full bg-elevated shadow-[var(--shadow-panel)]",
			"aria-hidden": true,
			children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", {
				ref: meterRef,
				className: "absolute inset-x-0 bottom-0 bg-accent transition-[height] duration-75 ease-linear",
				style: { height: "0%" }
			})
		})]
	});
}
var WAVES = [
	{
		id: "sine",
		label: "Sine"
	},
	{
		id: "triangle",
		label: "Tri"
	},
	{
		id: "sawtooth",
		label: "Saw"
	},
	{
		id: "square",
		label: "Square"
	}
];
function WaveIcon({ type }) {
	return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("svg", {
		viewBox: "0 0 28 24",
		className: "h-4 w-6",
		"aria-hidden": true,
		children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", {
			d: type === "sine" ? "M2 12 C6 12 6 4 10 4 C14 4 14 20 18 20 C22 20 22 12 26 12" : type === "triangle" ? "M2 20 L10 4 L18 20 L26 4" : type === "sawtooth" ? "M2 20 L2 4 L26 20" : "M2 20 L2 4 L14 4 L14 20 L26 20 L26 4",
			fill: "none",
			stroke: "currentColor",
			strokeWidth: "1.75",
			strokeLinecap: "round",
			strokeLinejoin: "round"
		})
	});
}
function WaveformSelect({ value, onChange, disabled }) {
	return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
		className: "flex flex-col gap-2",
		children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
			className: "text-[0.65rem] font-medium uppercase tracking-[0.14em] text-subtle",
			children: "Wave"
		}), /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", {
			role: "radiogroup",
			"aria-label": "Waveform",
			className: "grid grid-cols-4 gap-1 rounded-xl bg-elevated p-1 shadow-[var(--shadow-panel)]",
			children: WAVES.map((wave) => {
				const active = wave.id === value;
				return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("button", {
					type: "button",
					role: "radio",
					"aria-checked": active,
					disabled,
					onClick: () => onChange(wave.id),
					className: cn("flex min-h-11 flex-col items-center justify-center gap-1 rounded-lg px-1 py-1.5 text-[0.65rem] font-medium uppercase tracking-[0.08em] transition-[background-color,color] duration-150 ease-[cubic-bezier(0.22,1,0.36,1)]", active ? "bg-accent text-accent-fg" : "text-muted hover:bg-surface hover:text-fg"),
					children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)(WaveIcon, { type: wave.id }), wave.label]
				}, wave.id);
			})
		})]
	});
}
function Slider({ className, ...props }) {
	return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(Slider$1, {
		"data-slot": "slider",
		className: cn("relative flex w-full touch-none select-none items-center", className),
		...props,
		children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)(SliderTrack, {
			className: "relative h-1 w-full grow overflow-hidden rounded-full bg-elevated",
			children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(SliderRange, { className: "absolute h-full bg-accent" })
		}), /* @__PURE__ */ (0, import_jsx_runtime.jsx)(SliderThumb, { className: "block size-3.5 rounded-full bg-accent shadow-[var(--shadow-panel)] transition-[box-shadow,transform] duration-150 ease-[cubic-bezier(0.22,1,0.36,1)] hover:scale-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50" })]
	});
}
function formatHz(value) {
	return value >= 1e3 ? `${(value / 1e3).toFixed(1)} kHz` : `${Math.round(value)} Hz`;
}
function formatMs(value) {
	return value < 1 ? `${Math.round(value * 1e3)} ms` : `${value.toFixed(2)} s`;
}
function formatPct(value) {
	return `${Math.round(value * 100)}%`;
}
function HelixApp() {
	const waveform = useSynth((s) => s.waveform);
	const cutoff = useSynth((s) => s.cutoff);
	const resonance = useSynth((s) => s.resonance);
	const attack = useSynth((s) => s.attack);
	const decay = useSynth((s) => s.decay);
	const sustain = useSynth((s) => s.sustain);
	const release = useSynth((s) => s.release);
	const volume = useSynth((s) => s.volume);
	const octave = useSynth((s) => s.octave);
	const audioReady = useSynth((s) => s.audioReady);
	const pedal = useSynth((s) => s.pedal);
	const setWaveform = useSynth((s) => s.setWaveform);
	const setCutoff = useSynth((s) => s.setCutoff);
	const setResonance = useSynth((s) => s.setResonance);
	const setAttack = useSynth((s) => s.setAttack);
	const setDecay = useSynth((s) => s.setDecay);
	const setSustainLevel = useSynth((s) => s.setSustainLevel);
	const setRelease = useSynth((s) => s.setRelease);
	const setVolume = useSynth((s) => s.setVolume);
	const shiftOctave = useSynth((s) => s.shiftOctave);
	const low = midiToName(octaveBaseMidi(octave));
	const high = midiToName(octaveBaseMidi(octave) + 24);
	return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
		className: "flex min-h-dvh flex-col bg-bg text-fg",
		children: [/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("header", {
			className: "flex items-center justify-between gap-4 px-4 py-4 sm:px-6",
			children: [/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
				className: "min-w-0",
				children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", {
					className: "text-2xs font-medium uppercase tracking-[0.22em] text-subtle",
					children: "Analog keyboard"
				}), /* @__PURE__ */ (0, import_jsx_runtime.jsx)("h1", {
					className: "text-xl font-semibold tracking-tight sm:text-2xl",
					children: "Helix"
				})]
			}), /* @__PURE__ */ (0, import_jsx_runtime.jsx)(AuthSlot, {})]
		}), /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("main", {
			className: "mx-auto flex w-full max-w-6xl flex-1 flex-col gap-5 px-4 pb-8 sm:px-6",
			children: [/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("section", {
				className: "relative rounded-3xl bg-surface p-3 shadow-[var(--shadow-panel)] sm:p-4",
				children: [
					/* @__PURE__ */ (0, import_jsx_runtime.jsx)(EnableOverlay, {}),
					/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
						className: "flex flex-col gap-5 lg:grid lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1.4fr)] lg:items-stretch lg:gap-6",
						children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Oscilloscope, {}), /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
							className: "flex flex-col gap-5",
							children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)(WaveformSelect, {
								value: waveform,
								onChange: setWaveform,
								disabled: !audioReady
							}), /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
								className: "grid gap-5 sm:grid-cols-2",
								children: [/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("fieldset", {
									className: "flex flex-col gap-3",
									children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)("legend", {
										className: "text-2xs font-medium uppercase tracking-[0.14em] text-subtle",
										children: "Filter"
									}), /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
										className: "flex justify-around gap-2",
										children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Knob, {
											label: "Cutoff",
											value: cutoff,
											min: 80,
											max: 12e3,
											logarithmic: true,
											onChange: setCutoff,
											format: formatHz,
											disabled: !audioReady
										}), /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Knob, {
											label: "Reso",
											value: resonance,
											min: .1,
											max: 18,
											step: .1,
											onChange: setResonance,
											format: (v) => v.toFixed(1),
											disabled: !audioReady
										})]
									})]
								}), /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("fieldset", {
									className: "flex flex-col gap-3",
									children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)("legend", {
										className: "text-2xs font-medium uppercase tracking-[0.14em] text-subtle",
										children: "Envelope"
									}), /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
										className: "flex justify-around gap-1 sm:gap-2",
										children: [
											/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Knob, {
												label: "A",
												value: attack,
												min: .005,
												max: 2,
												logarithmic: true,
												onChange: setAttack,
												format: formatMs,
												disabled: !audioReady
											}),
											/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Knob, {
												label: "D",
												value: decay,
												min: .01,
												max: 2,
												logarithmic: true,
												onChange: setDecay,
												format: formatMs,
												disabled: !audioReady
											}),
											/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Knob, {
												label: "S",
												value: sustain,
												min: 0,
												max: 1,
												step: .01,
												onChange: setSustainLevel,
												format: formatPct,
												disabled: !audioReady
											}),
											/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Knob, {
												label: "R",
												value: release,
												min: .02,
												max: 3,
												logarithmic: true,
												onChange: setRelease,
												format: formatMs,
												disabled: !audioReady
											})
										]
									})]
								})]
							})]
						})]
					}),
					/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
						className: "mt-5 flex flex-col gap-4 border-t border-border/80 pt-4 sm:flex-row sm:items-center sm:justify-between",
						children: [/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
							className: "flex items-center gap-2",
							children: [
								/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Button, {
									type: "button",
									variant: "secondary",
									size: "icon-sm",
									"aria-label": "Octave down",
									disabled: octave <= 1,
									onClick: () => shiftOctave(-1),
									children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Minus, { className: "size-4" })
								}),
								/* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
									className: "min-w-24 text-center",
									children: [/* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", {
										className: "text-2xs font-medium uppercase tracking-[0.14em] text-subtle",
										children: "Octave"
									}), /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("p", {
										className: "font-mono text-sm tabular-nums text-fg",
										children: [
											low,
											"–",
											high
										]
									})]
								}),
								/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Button, {
									type: "button",
									variant: "secondary",
									size: "icon-sm",
									"aria-label": "Octave up",
									disabled: octave >= 6,
									onClick: () => shiftOctave(1),
									children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Plus, { className: "size-4" })
								}),
								/* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
									className: `ml-2 rounded-full px-2 py-1 text-2xs font-medium uppercase tracking-[0.12em] ${pedal ? "bg-accent text-accent-fg" : "bg-elevated text-subtle"}`,
									children: "Sustain"
								})
							]
						}), /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
							className: "flex min-w-0 items-center gap-3 sm:w-64",
							children: [
								/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Volume2, {
									className: "size-4 shrink-0 text-muted",
									"aria-hidden": true
								}),
								/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Slider, {
									min: 0,
									max: 1,
									step: .01,
									value: [volume],
									onValueChange: (v) => setVolume(v[0] ?? 0),
									disabled: !audioReady,
									"aria-label": "Volume"
								}),
								/* @__PURE__ */ (0, import_jsx_runtime.jsx)("span", {
									className: "w-10 text-right font-mono text-2xs tabular-nums text-muted",
									children: formatPct(volume)
								})
							]
						})]
					}),
					/* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", {
						className: "mt-4",
						children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Keyboard, {})
					})
				]
			}), /* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", {
				className: "px-1 text-center text-xs text-subtle sm:text-left",
				children: "Computer keys: Z row and Q row play notes. Space holds sustain. [ and ] shift octave. Esc silences all."
			})]
		})]
	});
}
function Home() {
	return /* @__PURE__ */ (0, import_jsx_runtime.jsx)(HelixApp, {});
}
//#endregion
export { Home as component };
