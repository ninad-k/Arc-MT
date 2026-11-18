/**
 * Mock-mode notes (ADR-0021): a small made-up notes folder for the galaxy page and a chat answer
 * that cites a note, so the page can be developed and tested without a backend.
 */
import type { CapturedNote, CitedNote, NoteText, NotesGraph, ToolCall } from './api';

const NOTES: { title: string; group: string; path: string; text: string }[] = [
	{
		title: 'Trading plan 2026',
		group: 'plans',
		path: 'plans/trading-plan-2026.md',
		text: '# Trading plan 2026\n\nGold, EURUSD and GBPUSD. Follow [[Risk per trade]] and the [[Gold news-day rule]].\n\n- London open to New York lunch.\n- Review every Friday.'
	},
	{
		title: 'Gold news-day rule',
		group: 'plans',
		path: 'plans/gold-news-day-rule.md',
		text: '# Gold news-day rule\n\nOn high-impact USD news days I do not open new **XAUUSD** trades from 30 minutes before the release until 30 minutes after it.\n\nSee [[Broker quirks]] and [[Lesson - NFP whipsaw]].'
	},
	{
		title: 'Risk per trade',
		group: 'rules',
		path: 'rules/risk-per-trade.md',
		text: '# Risk per trade\n\nNever risk more than 1% of equity on one trade. Daily loss limit: 2%.\n\nRelated: [[Revenge trading]].'
	},
	{
		title: 'Revenge trading',
		group: 'rules',
		path: 'rules/revenge-trading.md',
		text: '# Revenge trading\n\nAfter two losing trades in a row I stop for the day.'
	},
	{
		title: 'Broker quirks',
		group: 'brokers',
		path: 'brokers/broker-quirks.md',
		text: '# Broker quirks\n\nGold spread is about 10 points in London and 40-60 around US news.'
	},
	{
		title: 'Lesson - NFP whipsaw',
		group: 'lessons',
		path: 'lessons/lesson-nfp-whipsaw.md',
		text: '# Lesson - NFP whipsaw\n\nBought gold 2 minutes before NFP; stopped out, then it went back up. Led to the [[Gold news-day rule]].'
	}
];

const LINKS = [
	{ source: 0, target: 1 },
	{ source: 0, target: 2 },
	{ source: 1, target: 4 },
	{ source: 1, target: 5 },
	{ source: 2, target: 3 }
];

export function notesGraph(): NotesGraph {
	const degree = (id: number) => LINKS.filter((l) => l.source === id || l.target === id).length;
	return {
		dir: 'notes',
		nodes: NOTES.map((n, id) => ({
			id,
			title: n.title,
			group: n.group,
			path: n.path,
			excerpt: n.text.split('\n').slice(2).join(' ').slice(0, 200),
			degree: degree(id)
		})),
		links: LINKS,
		skipped: 0,
		error: null
	};
}

export function noteText(id: number): NoteText | null {
	const n = NOTES[id];
	return n ? { id, title: n.title, group: n.group, path: n.path, text: n.text } : null;
}

/** A chat answer from the notes, or null when the message is not about the user's notes. */
export function notesReply(message: string): { reply: string; tool_calls: ToolCall[]; notes: CitedNote[] } | null {
	const t = message.toLowerCase();
	if (!/\b(my (rule|rules|notes?|plan)|news[- ]day|in my notes)\b/.test(t)) return null;
	const call: ToolCall = { name: 'search_notes', arguments: { query: message }, ok: true, duration_ms: 3 };
	if (/gold|news/.test(t)) {
		return {
			reply: 'Your note Gold news-day rule says: no new gold trades from 30 minutes before a high-impact USD release until 30 minutes after it.',
			tool_calls: [call],
			notes: [{ id: 1, title: 'Gold news-day rule' }]
		};
	}
	return { reply: "Your notes don't cover that.", tool_calls: [call], notes: [] };
}

/** "Remember that ..." in mock mode: a new note in captures/, linked to nothing yet. */
export function addCapture(text: string): CapturedNote {
	const words = text.replace(/[^\p{L}\p{N}' ]+/gu, ' ').trim().split(/\s+/).slice(0, 6);
	const title = words.length ? words.join(' ').replace(/^./, (c) => c.toUpperCase()) : 'Note';
	const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'note';
	const path = `captures/2026-11-11-${slug}.md`;
	NOTES.push({ title, group: 'captures', path, text: `# ${title}

${text}

Captured on 2026-11-11 by Jarvis.` });
	return { id: NOTES.length - 1, title, path };
}
