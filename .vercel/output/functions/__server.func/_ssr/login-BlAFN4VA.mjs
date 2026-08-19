import { o as require_jsx_runtime } from "../_libs/@radix-ui/react-collection+[...].mjs";
import { _ as Link } from "../_libs/@tanstack/react-router+[...].mjs";
import { n as GROK_PROVIDERS } from "./router-Bzenk4j9.mjs";
import { i as signIn, t as Button } from "./button-BsdhwErW.mjs";
//#region node_modules/.nitro/vite/services/ssr/assets/login-BlAFN4VA.js
var import_jsx_runtime = require_jsx_runtime();
function GoogleMark() {
	return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("svg", {
		viewBox: "0 0 24 24",
		className: "size-4",
		"aria-hidden": true,
		children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", {
			fill: "currentColor",
			d: "M21.35 11.1h-9.18v2.96h5.27c-.23 1.5-1.78 4.4-5.27 4.4-3.17 0-5.76-2.62-5.76-5.86s2.59-5.86 5.76-5.86c1.8 0 3.01.77 3.7 1.43l2.52-2.43C16.54 4.04 14.47 3.1 12.17 3.1 7.36 3.1 3.5 6.98 3.5 12.8s3.86 9.7 8.67 9.7c5 0 8.3-3.51 8.3-8.46 0-.57-.06-1-.12-1.94Z"
		})
	});
}
function XMark() {
	return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("svg", {
		viewBox: "0 0 24 24",
		className: "size-4",
		"aria-hidden": true,
		children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", {
			fill: "currentColor",
			d: "M14.7 10.3 21.4 3h-1.6l-5.8 6.4L9.4 3H3.6l7 9.9L3.6 21h1.6l6.1-6.8 4.9 6.8h5.8l-7.3-10.7ZM12 13.3l-.7-1-5.6-7.8h2.4l4.5 6.4.7 1 5.9 8.2h-2.4L12 13.3Z"
		})
	});
}
function Login() {
	return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("main", {
		className: "grid min-h-dvh place-items-center bg-bg px-6 text-fg",
		children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", {
			className: "w-full max-w-sm rounded-3xl bg-surface p-6 shadow-[var(--shadow-panel)]",
			children: [
				/* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", {
					className: "text-2xs font-medium uppercase tracking-[0.22em] text-subtle",
					children: "Helix"
				}),
				/* @__PURE__ */ (0, import_jsx_runtime.jsx)("h1", {
					className: "mt-1 text-2xl font-semibold tracking-tight",
					children: "Sign in"
				}),
				/* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", {
					className: "mt-2 text-sm text-muted",
					children: "Save your place. The keyboard still plays as a guest."
				}),
				/* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", {
					className: "mt-6 space-y-2",
					children: GROK_PROVIDERS.map((provider) => /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(Button, {
						type: "button",
						variant: "secondary",
						className: "h-11 w-full justify-center",
						onClick: () => signIn(provider.providerId, { callbackURL: "/" }),
						children: [
							provider.idp === "google" ? /* @__PURE__ */ (0, import_jsx_runtime.jsx)(GoogleMark, {}) : /* @__PURE__ */ (0, import_jsx_runtime.jsx)(XMark, {}),
							"Continue with ",
							provider.label
						]
					}, provider.providerId))
				}),
				/* @__PURE__ */ (0, import_jsx_runtime.jsx)(Link, {
					to: "/",
					className: "mt-6 inline-flex text-sm text-muted transition-colors duration-150 hover:text-fg",
					children: "Back to the keyboard"
				})
			]
		})
	});
}
//#endregion
export { Login as component };
