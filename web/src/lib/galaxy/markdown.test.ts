import { describe, expect, it } from 'vitest';
import { parseNote } from './markdown';

describe('parseNote', () => {
	it('turns headings, lists and paragraphs into blocks', () => {
		const blocks = parseNote('# Title\n\nFirst line\nsame paragraph\n\n- one\n- two\n1. first\n\n## Sub');
		expect(blocks.map((b) => b.kind)).toEqual(['heading', 'paragraph', 'list', 'list', 'heading']);
		expect(blocks[0]).toMatchObject({ kind: 'heading', level: 1 });
		expect(blocks[2]).toMatchObject({ kind: 'list', ordered: false });
		expect(blocks[3]).toMatchObject({ kind: 'list', ordered: true });
		const list = blocks[2];
		if (list.kind !== 'list') throw new Error('list');
		expect(list.items.length).toBe(2);
	});

	it('marks bold, code and wikilinks inline, and never produces HTML', () => {
		const [p] = parseNote('Use **1%** and `max_lot`, see [[Risk per trade|risk]] <script>x</script>');
		if (p.kind !== 'paragraph') throw new Error('paragraph');
		expect(p.spans).toEqual([
			{ kind: 'text', text: 'Use ' },
			{ kind: 'bold', text: '1%' },
			{ kind: 'text', text: ' and ' },
			{ kind: 'code', text: 'max_lot' },
			{ kind: 'text', text: ', see ' },
			{ kind: 'link', text: 'risk', target: 'Risk per trade' },
			{ kind: 'text', text: ' <script>x</script>' }
		]);
	});

	it('keeps an indented continuation line in its list item', () => {
		const blocks = parseNote('1. Is there an event in the next hour? Check the calendar (and the rule for\n   gold).\n2. Is the spread normal?');
		expect(blocks.length).toBe(1);
		const list = blocks[0];
		if (list.kind !== 'list') throw new Error('list');
		expect(list.items.length).toBe(2);
		expect(list.items[0].map((s) => s.text).join('')).toBe('Is there an event in the next hour? Check the calendar (and the rule for gold).');
	});

	it('skips front matter', () => {
		const blocks = parseNote('---\ntags: [a]\n---\n# T\nbody');
		expect(blocks[0]).toMatchObject({ kind: 'heading' });
	});

	it('stays fast on a long line of unclosed markers', () => {
		const line = '[[a '.repeat(20000);
		const t0 = performance.now();
		const [p] = parseNote(line);
		expect(performance.now() - t0).toBeLessThan(500);
		if (p.kind !== 'paragraph') throw new Error('paragraph');
		expect(p.spans).toEqual([{ kind: 'text', text: line.trim() }]);
	});

	it('keeps an unclosed marker as text', () => {
		const [p] = parseNote('a **b and [[c');
		if (p.kind !== 'paragraph') throw new Error('paragraph');
		expect(p.spans).toEqual([{ kind: 'text', text: 'a **b and [[c' }]);
	});
});
