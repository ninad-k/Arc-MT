/**
 * The page script of the 3D model sandbox (built into `static/sandbox/model.html` by
 * `scripts/build-sandbox.mjs`).
 *
 * The page runs inside `<iframe sandbox="allow-scripts">` under a policy with no network access
 * (`connect-src` allows only local `blob:` and `data:` reads): an opaque origin with no access to
 * the app. It never runs
 * the model's code itself. That runs in a Web Worker (`sandbox-worker.ts`, created from a blob, so
 * it keeps this page's policy) drawing to an OffscreenCanvas. If the worker stops answering (a
 * model that never stops), this page terminates it: neither this page nor the app can be frozen.
 *
 * It talks to the app only through `postMessage` (see `protocol.ts`), and only with its parent.
 */
import type { FromSandbox, ToSandbox } from './protocol';

/** The worker's source, inlined at build time. */
declare const __SANDBOX_WORKER__: string;

const parentWindow = window.parent;

// Only ever run as a sandboxed frame (opaque origin, so `self.origin` is "null"). Opened directly,
// or framed without the sandbox by another site, the page does nothing at all: otherwise a
// page that frames it could run code on this app's origin.
if (self.origin !== 'null' || parentWindow === window) {
	document.body.textContent = 'This page only runs inside Arc MT.';
	throw new Error('the 3D sandbox page only runs in a sandboxed frame');
}

/** Silence from the worker that counts as stuck: while building Claude's code, while parsing an
 * uploaded file (a large file takes longer), then once running. */
const BUILD_LIMIT_MS = 10_000;
const PARSE_LIMIT_MS = 30_000;
const SILENCE_LIMIT_MS = 4_000;
let startLimit = BUILD_LIMIT_MS;

function post(msg: FromSandbox): void {
	// The app's origin cannot be named from an opaque origin, so '*': the messages carry no
	// secrets, and the app checks that they come from this frame.
	// nosemgrep: javascript.browser.security.wildcard-postmessage-configuration.wildcard-postmessage-configuration
	parentWindow.postMessage(msg, '*');
}

const canvas = document.createElement('canvas');
document.body.appendChild(canvas);
const offscreen = canvas.transferControlToOffscreen();
const workerUrl = URL.createObjectURL(new Blob([__SANDBOX_WORKER__], { type: 'text/javascript' }));
let worker: Worker | null = new Worker(workerUrl);
let lastHeard = performance.now();
let running = false;
let ready = false;

const size = () => ({ width: window.innerWidth || 1, height: window.innerHeight || 1 });
worker.postMessage({ type: 'init', canvas: offscreen, dpr: window.devicePixelRatio || 1, ...size() }, [offscreen]);

worker.onmessage = (event: MessageEvent) => {
	lastHeard = performance.now();
	const msg = event.data as FromSandbox;
	if (msg?.type === 'ready') ready = true;
	if (msg && typeof msg === 'object' && typeof msg.type === 'string') post(msg);
};
worker.onerror = (event: ErrorEvent) => {
	event.preventDefault();
	post({ type: 'error', message: event.message || 'the model stopped with an error' });
};

function stop(reason: string): void {
	worker?.terminate();
	worker = null;
	URL.revokeObjectURL(workerUrl);
	post({ type: 'error', message: reason });
}

// The watchdog: a worker that stops sending its heartbeat is terminated. Hidden pages pause it.
setInterval(() => {
	if (!worker || !running || document.hidden) return;
	const quiet = performance.now() - lastHeard;
	if (quiet > (ready ? SILENCE_LIMIT_MS : startLimit)) {
		stop('it stopped responding (it may never finish), so it was stopped');
	}
}, 500);

// The sender is checked by identity (`event.source`), which is stricter than an origin check:
// this opaque-origin frame cannot name the app's origin.
// nosemgrep: javascript.browser.security.insufficient-postmessage-origin-validation.insufficient-postmessage-origin-validation
window.addEventListener('message', (event: MessageEvent) => {
	if (event.source !== parentWindow || !worker) return;
	const msg = event.data as ToSandbox | null;
	if (!msg || typeof msg !== 'object') return;
	if (msg.type === 'run' && typeof msg.code === 'string') {
		running = true;
		ready = false;
		startLimit = BUILD_LIMIT_MS;
		lastHeard = performance.now();
		worker.postMessage({ type: 'run', code: msg.code });
	} else if ((msg.type === 'load' || msg.type === 'relief') && msg.bytes instanceof ArrayBuffer) {
		running = true;
		ready = false;
		startLimit = PARSE_LIMIT_MS;
		lastHeard = performance.now();
		const overlay = typeof msg.overlay === 'string' ? msg.overlay : null;
		const format = msg.type === 'load' ? String(msg.format) : undefined;
		worker.postMessage({ type: msg.type, format, bytes: msg.bytes, overlay }, [msg.bytes]);
	} else if (msg.type === 'reset-view') worker.postMessage({ type: 'reset-view' });
	else if (msg.type === 'pause') worker.postMessage({ type: 'pause', paused: msg.paused === true });
});

document.addEventListener('visibilitychange', () => {
	worker?.postMessage({ type: 'pause', paused: document.hidden });
	if (!document.hidden) lastHeard = performance.now();
});

window.addEventListener('resize', () => worker?.postMessage({ type: 'resize', ...size() }));

// Orbit and zoom: drag with one pointer, pinch with two, scroll to zoom, double-click to reset.
const pointers = new Map<number, { x: number; y: number }>();
let pinch = 0;
canvas.addEventListener('pointerdown', (e) => {
	canvas.setPointerCapture(e.pointerId);
	pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
});
canvas.addEventListener('pointermove', (e) => {
	const prev = pointers.get(e.pointerId);
	if (!prev || !worker) return;
	pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
	if (pointers.size === 1) {
		worker.postMessage({ type: 'orbit', dx: e.clientX - prev.x, dy: e.clientY - prev.y });
	} else if (pointers.size === 2) {
		const [a, b] = [...pointers.values()];
		const d = Math.hypot(a.x - b.x, a.y - b.y);
		if (pinch > 0 && d > 0) worker.postMessage({ type: 'zoom', factor: pinch / d });
		pinch = d;
	}
});
const lift = (e: PointerEvent) => {
	pointers.delete(e.pointerId);
	if (pointers.size < 2) pinch = 0;
};
canvas.addEventListener('pointerup', lift);
canvas.addEventListener('pointercancel', lift);
canvas.addEventListener(
	'wheel',
	(e) => {
		e.preventDefault();
		worker?.postMessage({ type: 'zoom', factor: Math.exp(e.deltaY * 0.001) });
	},
	{ passive: false }
);
canvas.addEventListener('dblclick', () => worker?.postMessage({ type: 'reset-view' }));
