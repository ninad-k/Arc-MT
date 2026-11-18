/**
 * Just enough markdown to read a note in the galaxy side panel: headings, bullet and numbered
 * lists, paragraphs, and inline bold, code and [[wikilinks]]. It returns plain data that the page
 * renders as text nodes, so nothing in a note can ever become HTML or script (ADR-0021).
 */

export type Span =
	| { kind: 'text'; text: string }
	| { kind: 'bold'; text: string }
	| { kind: 'code'; text: string }
	/** `[[target]]` or `[[target|label]]`; `text` is what is shown. */
	| { kind: 'link'; text: string; target: string };

export type Block =
	| { kind: 'heading'; level: number; spans: Span[] }
	| { kind: 'paragraph'; spans: Span[] }
	| { kind: 'list'; ordered: boolean; items: Span[][] };

/** Inline markers, in the order they are tried at each position. */
const MARKERS: { open: string; close: string; make: (inner: string) => Span | null }[] = [
	{
		open: '[[',
		close: ']]',
		make: (inner) => {
			const [target, label] = inner.split('|');
			const t = target.split('#')[0].trim();
			return t ? { kind: 'link', text: (label ?? target).trim(), target: t } : null;
		}
	},
	{ open: '**', close: '**', make: (inner) => (inner ? { kind: 'bold', text: inner } : null) },
	{ open: '`', close: '`', make: (inner) => (inner ? { kind: 'code', text: inner } : null) }
];

export function parseInline(line: string): Span[] {
	const spans: Span[] = [];
	let text = '';
	let i = 0;
	const flush = () => {
		if (text) spans.push({ kind: 'text', text });
		text = '';
	};
	// A marker with no closing pair later in the line is never tried again (keeps a line full of
	// unclosed `[[` linear instead of quadratic).
	const unclosed = new Set<string>();
	outer: while (i < line.length) {
		for (const m of MARKERS) {
			if (unclosed.has(m.open) || !line.startsWith(m.open, i)) continue;
			const end = line.indexOf(m.close, i + m.open.length);
			if (end < 0) {
				unclosed.add(m.open);
				continue;
			}
			const span = m.make(line.slice(i + m.open.length, end));
			if (!span) continue;
			flush();
			spans.push(span);
			i = end + m.close.length;
			continue outer;
		}
		text += line[i];
		i++;
	}
	flush();
	return spans;
}

export function parseNote(source: string): Block[] {
	let lines = source.replace(/^﻿/, '').split(/\r?\n/);
	if (lines[0]?.trim() === '---') {
		const end = lines.findIndex((l, i) => i > 0 && l.trim() === '---');
		if (end > 0) lines = lines.slice(end + 1);
	}
	const blocks: Block[] = [];
	let para: string[] = [];
	const flushPara = () => {
		if (para.length) blocks.push({ kind: 'paragraph', spans: parseInline(para.join(' ')) });
		para = [];
	};
	/** The raw source of the last list item, so an indented continuation line can join it. */
	let lastItem: string | null = null;
	for (const raw of lines) {
		const line = raw.trim();
		if (!line) {
			flushPara();
			lastItem = null;
			continue;
		}
		const last = blocks.at(-1);
		if (/^\s+/.test(raw) && lastItem !== null && para.length === 0 && last?.kind === 'list') {
			lastItem = `${lastItem} ${line}`;
			last.items[last.items.length - 1] = parseInline(lastItem);
			continue;
		}
		const heading = /^(#{1,6})\s+(.*)$/.exec(line);
		const bullet = /^[-*+]\s+(.*)$/.exec(line);
		const numbered = /^\d+[.)]\s+(.*)$/.exec(line);
		if (heading) {
			flushPara();
			lastItem = null;
			blocks.push({ kind: 'heading', level: heading[1].length, spans: parseInline(heading[2]) });
		} else if (bullet || numbered) {
			flushPara();
			const ordered = !!numbered;
			lastItem = (bullet ?? numbered)![1];
			const item = parseInline(lastItem);
			if (last?.kind === 'list' && last.ordered === ordered) last.items.push(item);
			else blocks.push({ kind: 'list', ordered, items: [item] });
		} else {
			lastItem = null;
			para.push(line);
		}
	}
	flushPara();
	return blocks;
}
