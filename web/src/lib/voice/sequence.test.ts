import { describe, expect, it } from 'vitest';
import { chunkForSpeech, isMuted, playInSequence } from './sequence';
import { MAX_SPEAK } from './text';

describe('chunkForSpeech', () => {
	it('keeps a short reply as one chunk and collapses whitespace', () => {
		expect(chunkForSpeech('Gold is at 2400.5,\n  sir.')).toEqual(['Gold is at 2400.5, sir.']);
	});

	it('packs whole sentences up to about the target length', () => {
		const s = (n: number) => `${'w'.repeat(n - 1)}.`;
		const text = [s(60), s(60), s(60), s(60)].join(' ');
		const chunks = chunkForSpeech(text, 170);
		expect(chunks).toEqual([`${s(60)} ${s(60)}`, `${s(60)} ${s(60)}`]);
		for (const c of chunks) expect(c.endsWith('.')).toBe(true);
	});

	it('never splits inside a sentence, even a long one', () => {
		const long = `${'word '.repeat(50).trim()}.`; // ~250 characters
		const chunks = chunkForSpeech(`Short one. ${long} Last one.`, 170);
		expect(chunks).toEqual(['Short one.', long, 'Last one.']);
	});

	it('splits a sentence longer than the server limit at a word', () => {
		const huge = 'lorem '.repeat(150).trim(); // ~900 characters, no full stop
		const chunks = chunkForSpeech(huge);
		expect(chunks.length).toBe(2);
		for (const c of chunks) expect(c.length).toBeLessThanOrEqual(MAX_SPEAK);
		expect(chunks.join(' ')).toBe(huge);
	});

	it('does not split decimals or stop at question and exclamation marks wrongly', () => {
		expect(chunkForSpeech('Buy 0.1 gold? Yes! Done.', 5)).toEqual(['Buy 0.1 gold?', 'Yes!', 'Done.']);
	});

	it('caps the number of chunks', () => {
		const text = Array.from({ length: 20 }, (_, i) => `Sentence number ${i} is here.`).join(' ');
		expect(chunkForSpeech(text, 30, 3).length).toBe(3);
	});

	it('returns nothing for empty text', () => {
		expect(chunkForSpeech('   ')).toEqual([]);
	});
});

describe('isMuted', () => {
	it('reads mute=1 from the query string only', () => {
		expect(isMuted('?mute=1')).toBe(true);
		expect(isMuted('?a=b&mute=1')).toBe(true);
		expect(isMuted('?mute=0')).toBe(false);
		expect(isMuted('')).toBe(false);
	});
});

/** A fake player and fetcher that log every step. */
function harness() {
	const log: string[] = [];
	const resolvers = new Map<string, () => void>();
	const fetchChunk = (text: string) => {
		log.push(`fetch ${text}`);
		return Promise.resolve(new Blob([text]));
	};
	const play = async (blob: Blob) => {
		const text = await blob.text();
		log.push(`play ${text}`);
		let finish!: () => void;
		const done = new Promise<void>((r) => (finish = r));
		resolvers.set(text, () => {
			log.push(`end ${text}`);
			finish();
		});
		return { done, stop: () => resolvers.get(text)?.() };
	};
	const tick = () => new Promise((r) => setTimeout(r, 0));
	return { log, resolvers, fetchChunk, play, tick };
}

describe('playInSequence', () => {
	it('fetches the next chunk while the current one plays and resolves after the last', async () => {
		const h = harness();
		const ctrl = new AbortController();
		let settled = false;
		const run = playInSequence(['a', 'b', 'c'], h.fetchChunk, h.play, ctrl.signal).then((ok) => {
			settled = true;
			return ok;
		});
		await h.tick();
		// 'b' was requested before 'a' finished playing.
		expect(h.log).toEqual(['fetch a', 'fetch b', 'play a']);
		h.resolvers.get('a')!();
		await h.tick();
		expect(h.log.slice(3)).toEqual(['end a', 'fetch c', 'play b']);
		h.resolvers.get('b')!();
		await h.tick();
		expect(settled).toBe(false);
		h.resolvers.get('c')!();
		expect(await run).toBe(true);
		expect(h.log.at(-1)).toBe('end c');
		expect(h.log.filter((l) => l.startsWith('fetch')).length).toBe(3);
	});

	it('stops at once when aborted and plays nothing more', async () => {
		const h = harness();
		const ctrl = new AbortController();
		const current: { stop(): void }[] = [];
		const run = playInSequence(
			['a', 'b'],
			h.fetchChunk,
			async (blob) => {
				const p = await h.play(blob);
				current.push(p);
				return p;
			},
			ctrl.signal
		);
		await h.tick();
		ctrl.abort();
		current[0].stop();
		expect(await run).toBe(false);
		expect(h.log.some((l) => l === 'play b')).toBe(false);
	});

	it('stops a chunk that finished decoding after the abort, before it is heard', async () => {
		const ctrl = new AbortController();
		const stopped: string[] = [];
		let release!: () => void;
		const decoding = new Promise<void>((r) => (release = r));
		const run = playInSequence(
			['a', 'b'],
			(text) => Promise.resolve(new Blob([text])),
			async (blob) => {
				const text = await blob.text();
				await decoding; // still decoding when the user barges in
				return { done: new Promise<void>(() => {}), stop: () => stopped.push(text) };
			},
			ctrl.signal
		);
		await new Promise((r) => setTimeout(r, 0));
		ctrl.abort();
		release();
		expect(await run).toBe(false);
		expect(stopped).toEqual(['a']);
	});

	it('rejects when a chunk cannot be fetched', async () => {
		const h = harness();
		const failing = (text: string) => (text === 'b' ? Promise.reject(new Error('down')) : h.fetchChunk(text));
		const run = playInSequence(['a', 'b'], failing, h.play, new AbortController().signal);
		await h.tick();
		h.resolvers.get('a')!();
		await expect(run).rejects.toThrow('down');
	});
});
