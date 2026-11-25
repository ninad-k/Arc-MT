/**
 * What the UI shows and logs when a screen crashes or cannot load (`hooks.client.ts` and
 * `+error.svelte`). Pure, so it is unit-tested; those files only call it.
 */

import { ApiError, describeError } from './errors';

/** Plain text for the trader. Raw exception text and stacks stay in the console. */
export function crashMessage(error: unknown, status?: number): string {
	if (error instanceof ApiError) return describeError(error);
	if (status === 404) return 'There is no page at this address.';
	return 'Something went wrong on this screen.';
}

/** The console entry for a crash: the route and what was thrown. */
export function crashReport(error: unknown, route: string | null): Record<string, unknown> {
	const report: Record<string, unknown> = { route };
	if (error instanceof Error) {
		report.name = error.name;
		report.message = error.message;
		if (error.stack) report.stack = error.stack;
		if (error instanceof ApiError) {
			report.code = error.code;
			report.status = error.status;
		}
	} else {
		report.thrown = safeString(error);
	}
	return report;
}

function safeString(v: unknown): string {
	try {
		return typeof v === 'string' ? v : (JSON.stringify(v) ?? String(v));
	} catch {
		return Object.prototype.toString.call(v);
	}
}
