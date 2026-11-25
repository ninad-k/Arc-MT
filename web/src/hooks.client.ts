import type { HandleClientError } from '@sveltejs/kit/hooks';
import { crashMessage, crashReport } from '#lib/crash.ts';

/**
 * Every error SvelteKit catches on the client (a failed load, an unknown route, a page that throws
 * while rendering): log it with context. An unexpected error gets a message without raw exception
 * text; app and framework errors (such as a 404) keep SvelteKit's own safe message and status.
 * +error.svelte then shows it.
 */
export const handleError: HandleClientError = ({ kind, error, event }) => {
	console.error('[arc-mt] navigation failed', {
		kind,
		url: event.url.pathname,
		...crashReport(error, event.route.id)
	});
	if (kind === 'unknown') return { message: crashMessage(error) };
};
