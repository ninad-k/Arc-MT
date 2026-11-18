/**
 * Messages between the app and the 3D model sandbox (spec-assistant-mode, ADR-0022).
 *
 * The sandbox page (`/sandbox/model.html`) runs in an iframe with `sandbox="allow-scripts"` and
 * its own Content-Security-Policy: an opaque origin, no network, no access to the app. The only
 * way in or out is `postMessage`, with the small set of messages below. Anything else, or anything
 * from another window, is ignored on both sides.
 */

/** The 3D file formats the sandbox can load (spec-model-uploads). */
export type FileFormat = 'glb' | 'gltf' | 'obj' | 'stl' | 'ply';

/** What the sandbox found in a loaded file: part names and overall size. */
export interface ModelInfo {
	parts: string[];
	size: [number, number, number];
}

/** App to sandbox. */
export type ToSandbox =
	| { type: 'run'; code: string; title: string }
	/** An uploaded 3D file (the bytes are transferred), with Claude's change to it, if any. */
	| { type: 'load'; format: FileFormat; bytes: ArrayBuffer; overlay?: string | null }
	/** A picture shown as a raised relief, with Claude's change to it, if any. */
	| { type: 'relief'; bytes: ArrayBuffer; overlay?: string | null }
	| { type: 'reset-view' }
	| { type: 'pause'; paused: boolean };

/** Sandbox to app. */
export type FromSandbox =
	| { type: 'loaded' }
	| { type: 'ready'; triangles: number; info?: ModelInfo }
	| { type: 'error'; message: string }
	/** Sent about once a second while the page is alive (the watchdog listens for it). */
	| { type: 'tick'; fps: number };

function parseInfo(v: unknown): ModelInfo | undefined {
	if (typeof v !== 'object' || v === null) return undefined;
	const o = v as Record<string, unknown>;
	if (!Array.isArray(o.parts) || !Array.isArray(o.size) || o.size.length !== 3) return undefined;
	const size = o.size.map(Number);
	if (size.some((n) => !Number.isFinite(n) || n < 0)) return undefined;
	const parts = o.parts.filter((p): p is string => typeof p === 'string').slice(0, 200).map((p) => p.slice(0, 80));
	return { parts, size: size as [number, number, number] };
}

/** Read a message from the sandbox; `null` for anything that is not one of ours. */
export function parseFromSandbox(data: unknown): FromSandbox | null {
	if (typeof data !== 'object' || data === null) return null;
	const d = data as Record<string, unknown>;
	switch (d.type) {
		case 'loaded':
			return { type: 'loaded' };
		case 'ready': {
			const info = parseInfo(d.info);
			return { type: 'ready', triangles: typeof d.triangles === 'number' ? d.triangles : 0, ...(info ? { info } : {}) };
		}
		case 'error':
			return { type: 'error', message: typeof d.message === 'string' ? d.message.slice(0, 300) : 'unknown error' };
		case 'tick':
			return { type: 'tick', fps: typeof d.fps === 'number' ? d.fps : 0 };
		default:
			return null;
	}
}

/** The format of a 3D file from its name, or null when the sandbox cannot load it. */
export function fileFormat(name: string): FileFormat | null {
	const ext = name.toLowerCase().split('.').pop() ?? '';
	return (['glb', 'gltf', 'obj', 'stl', 'ply'] as const).find((f) => f === ext) ?? null;
}

/** How long the sandbox may stay silent before the app gives up on it. */
export const SILENCE_LIMIT_MS = 5000;
/** How long building the scene may take, from `run` to `ready` or `error` (the sandbox's own
 * watchdog stops a stuck build sooner; parsing a large upload may take up to 30 s). */
export const START_LIMIT_MS = 40000;

/**
 * The watchdog's verdict at time `now`: is the sandbox still alive? `lastHeard` is when any
 * message last arrived; `startedAt` when the code was sent; `ready` whether it reported back.
 */
export function watchdog(
	now: number,
	s: { startedAt: number; lastHeard: number; ready: boolean }
): 'ok' | 'stuck' {
	if (!s.ready && now - s.startedAt > START_LIMIT_MS) return 'stuck';
	if (s.ready && now - s.lastHeard > SILENCE_LIMIT_MS) return 'stuck';
	return 'ok';
}
