import { describe, expect, it } from 'vitest';
import { parseFromSandbox, SILENCE_LIMIT_MS, START_LIMIT_MS, watchdog } from './protocol';

describe('parseFromSandbox', () => {
	it('accepts only the known messages, with safe fields', () => {
		expect(parseFromSandbox({ type: 'loaded' })).toEqual({ type: 'loaded' });
		expect(parseFromSandbox({ type: 'ready', triangles: 1200 })).toEqual({ type: 'ready', triangles: 1200 });
		expect(parseFromSandbox({ type: 'ready', triangles: 'lots' })).toEqual({ type: 'ready', triangles: 0 });
		expect(parseFromSandbox({ type: 'tick', fps: 60 })).toEqual({ type: 'tick', fps: 60 });
		const long = parseFromSandbox({ type: 'error', message: 'x'.repeat(5000) });
		expect(long?.type === 'error' && long.message.length).toBe(300);
	});

	it('ignores anything else', () => {
		for (const bad of [null, undefined, 'loaded', 42, {}, { type: 'run', code: 'x' }, { type: 'navigate', url: '/' }]) {
			expect(parseFromSandbox(bad)).toBeNull();
		}
	});
});

describe('watchdog', () => {
	const t0 = 1_000_000;
	it('gives a model time to build, then needs a heartbeat', () => {
		expect(watchdog(t0 + START_LIMIT_MS - 1, { startedAt: t0, lastHeard: t0, ready: false })).toBe('ok');
		expect(watchdog(t0 + START_LIMIT_MS + 1, { startedAt: t0, lastHeard: t0, ready: false })).toBe('stuck');
		const heard = t0 + 2000;
		expect(watchdog(heard + SILENCE_LIMIT_MS - 1, { startedAt: t0, lastHeard: heard, ready: true })).toBe('ok');
		expect(watchdog(heard + SILENCE_LIMIT_MS + 1, { startedAt: t0, lastHeard: heard, ready: true })).toBe('stuck');
	});
});
