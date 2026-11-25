import { describe, expect, it } from 'vitest';
import { crashMessage, crashReport } from './crash';
import { ApiError } from './errors';

describe('crashMessage', () => {
	it('never shows raw exception text', () => {
		const msg = crashMessage(new TypeError("Cannot read properties of undefined (reading 'x')"));
		expect(msg).toBe('Something went wrong on this screen.');
		expect(msg).not.toContain('undefined');
	});

	it('uses the plain-language text of an API error', () => {
		expect(crashMessage(new ApiError('network', 'fetch failed'))).toBe(
			'Cannot reach the Arc MT server.'
		);
	});

	it('names a missing page', () => {
		expect(crashMessage(new Error('Not found: /nope'), 404)).toBe(
			'There is no page at this address.'
		);
	});
});

describe('crashReport', () => {
	it('keeps the context and the details of an Error for the console', () => {
		const err = new RangeError('bad index');
		const r = crashReport(err, '/settings');
		expect(r).toMatchObject({
			route: '/settings',
			name: 'RangeError',
			message: 'bad index'
		});
		expect(typeof r.stack).toBe('string');
	});

	it('adds the code and status of an API error', () => {
		const r = crashReport(new ApiError('rate_limited', 'slow down', 429), '/');
		expect(r).toMatchObject({ code: 'rate_limited', status: 429 });
	});

	it('copes with thrown non-errors, including ones that cannot be serialised', () => {
		expect(crashReport('boom', null).thrown).toBe('boom');
		expect(crashReport({ a: 1 }, null).thrown).toBe('{"a":1}');
		const cyclic: Record<string, unknown> = {};
		cyclic.self = cyclic;
		expect(crashReport(cyclic, null).thrown).toBe('[object Object]');
		expect(crashReport(undefined, null).thrown).toBe('undefined');
	});
});
