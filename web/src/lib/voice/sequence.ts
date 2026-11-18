/**
 * Speaking a long reply (FL-5): split it into sentence-packed chunks and play them in order,
 * fetching the next chunk while the current one plays so there is no gap between them.
 * Pure, so it is unit tested; the browser parts are passed in.
 *
 * The rule: never cut inside a sentence to start sooner. A voice that pauses in the middle of a
 * sentence sounds broken; a second more before the first word does not.
 */
import { MAX_SPEAK } from './text';

/** About how many characters one chunk holds. */
export const CHUNK_TARGET = 170;
/** At most this many chunks are spoken (about 1,400 characters). */
export const MAX_CHUNKS = 8;

/** Sentences of `text`: split after `.`, `?` or `!` followed by a space (decimals stay whole). */
function sentences(text: string): string[] {
	return text.split(/(?<=[.!?])\s+/).filter((s) => s.length > 0);
}

/** Split a sentence longer than the server limit at word boundaries. */
function splitLong(sentence: string): string[] {
	if (sentence.length <= MAX_SPEAK) return [sentence];
	const pieces: string[] = [];
	let current = '';
	for (const word of sentence.split(' ')) {
		const next = current ? `${current} ${word}` : word;
		if (next.length > MAX_SPEAK && current) {
			pieces.push(current);
			current = word.slice(0, MAX_SPEAK);
		} else {
			current = next.slice(0, MAX_SPEAK);
		}
	}
	if (current) pieces.push(current);
	return pieces;
}

/** Whole sentences packed into chunks of about `target` characters, at most `maxChunks`. */
export function chunkForSpeech(text: string, target = CHUNK_TARGET, maxChunks = MAX_CHUNKS): string[] {
	const clean = text.replace(/\s+/g, ' ').trim();
	if (!clean) return [];
	const chunks: string[] = [];
	let current = '';
	for (const piece of sentences(clean).flatMap(splitLong)) {
		const joined = current ? `${current} ${piece}` : piece;
		if (current && joined.length > target) {
			chunks.push(current);
			current = piece;
		} else {
			current = joined;
		}
	}
	if (current) chunks.push(current);
	return chunks.slice(0, Math.max(1, maxChunks));
}

/** True when the page URL has `mute=1`: that tab never makes a sound (tests, background tabs). */
export function isMuted(search: string = typeof location === 'undefined' ? '' : location.search): boolean {
	return new URLSearchParams(search).get('mute') === '1';
}

export interface SequencePlayback {
	done: Promise<void>;
	stop(): void;
}

/**
 * Play `chunks` in order. Chunk n+1 is requested as soon as chunk n starts playing. Resolves
 * `true` after the last chunk has finished, `false` when `signal` aborted it; rejects when a
 * chunk cannot be fetched or played.
 */
export async function playInSequence(
	chunks: readonly string[],
	fetchChunk: (text: string, signal: AbortSignal) => Promise<Blob>,
	play: (blob: Blob) => Promise<SequencePlayback>,
	signal: AbortSignal
): Promise<boolean> {
	if (chunks.length === 0) return false;
	let next = fetchChunk(chunks[0], signal);
	for (let i = 0; i < chunks.length; i++) {
		const blob = await next;
		if (signal.aborted) return false;
		if (i + 1 < chunks.length) {
			next = fetchChunk(chunks[i + 1], signal);
			// Awaited on the next turn of the loop; this only stops an "unhandled rejection"
			// warning when playback is stopped before then.
			next.catch(() => {});
		}
		const playback = await play(blob);
		// Stopped while this chunk was being decoded: silence it before it is heard.
		if (signal.aborted) {
			playback.stop();
			return false;
		}
		await playback.done;
		if (signal.aborted) return false;
	}
	return true;
}
