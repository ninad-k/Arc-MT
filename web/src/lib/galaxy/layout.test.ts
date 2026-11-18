import { describe, expect, it } from 'vitest';
import { graphKey, layout, neighbours } from './layout';

const nodes = Array.from({ length: 12 }, (_, i) => ({ id: i, group: i < 6 ? 'plans' : 'lessons' }));
const links = [
	{ source: 0, target: 1 },
	{ source: 1, target: 2 },
	{ source: 2, target: 0 },
	{ source: 6, target: 7 }
];

const dist = (p: Float32Array, a: number, b: number) =>
	Math.hypot(p[a * 3] - p[b * 3], p[a * 3 + 1] - p[b * 3 + 1], p[a * 3 + 2] - p[b * 3 + 2]);

describe('layout', () => {
	it('gives every node a finite position', () => {
		const p = layout(nodes, links);
		expect(p.length).toBe(nodes.length * 3);
		expect([...p].every(Number.isFinite)).toBe(true);
	});

	it('is deterministic for the same input and seed', () => {
		expect([...layout(nodes, links, { seed: 7 })]).toEqual([...layout(nodes, links, { seed: 7 })]);
		expect([...layout(nodes, links, { seed: 7 })]).not.toEqual([...layout(nodes, links, { seed: 8 })]);
	});

	it('pulls linked notes closer than unlinked ones', () => {
		const p = layout(nodes, links);
		const linked = (dist(p, 0, 1) + dist(p, 1, 2) + dist(p, 6, 7)) / 3;
		let unlinked = 0;
		let n = 0;
		for (let a = 0; a < nodes.length; a++)
			for (let b = a + 1; b < nodes.length; b++) {
				if (links.some((l) => (l.source === a && l.target === b) || (l.source === b && l.target === a))) continue;
				unlinked += dist(p, a, b);
				n++;
			}
		expect(linked).toBeLessThan(unlinked / n);
	});

	it('keeps notes apart (no two in the same place)', () => {
		const p = layout(nodes, links);
		for (let a = 0; a < nodes.length; a++)
			for (let b = a + 1; b < nodes.length; b++) expect(dist(p, a, b)).toBeGreaterThan(0.5);
	});

	it('handles no nodes and links to unknown ids', () => {
		expect(layout([], []).length).toBe(0);
		expect(() => layout(nodes, [{ source: 0, target: 99 }])).not.toThrow();
	});
});

describe('neighbours and graphKey', () => {
	it('lists the notes linked to one note', () => {
		expect([...neighbours(1, links)].sort()).toEqual([0, 2]);
		expect([...neighbours(11, links)]).toEqual([]);
	});

	it('changes when the graph changes', () => {
		const a = graphKey(nodes.map((n) => ({ ...n, title: `n${n.id}` })), links);
		const b = graphKey(nodes.map((n) => ({ ...n, title: `n${n.id}` })), links.slice(1));
		expect(a).not.toBe(b);
		expect(a).toBe(graphKey(nodes.map((n) => ({ ...n, title: `n${n.id}` })), links));
	});
});
