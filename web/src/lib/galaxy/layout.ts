/**
 * A small 3D force layout for the notes galaxy (ADR-0021): repulsion between every pair of notes,
 * springs along links, a pull towards the centre, and a gentle pull of each folder (group)
 * towards its own direction so colours form clusters. Seeded, so the same notes always land in
 * the same places. Pure, so it is unit tested; the scene only reads the positions.
 */

export interface LayoutNode {
	id: number;
	group: string;
}

export interface LayoutLink {
	source: number;
	target: number;
}

export interface LayoutOptions {
	seed?: number;
	/** Fixed number of simulation steps; by default fewer for big graphs. */
	iterations?: number;
}

/** mulberry32: a tiny seeded random generator. */
function random(seed: number): () => number {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = a;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

function stepsFor(n: number): number {
	if (n <= 200) return 300;
	if (n <= 600) return 120;
	return 40;
}

/** Positions as `[x0, y0, z0, x1, ...]`, in the order of `nodes`. */
export function layout(nodes: readonly LayoutNode[], links: readonly LayoutLink[], opts: LayoutOptions = {}): Float32Array {
	const n = nodes.length;
	const pos = new Float32Array(n * 3);
	if (n === 0) return pos;
	const rand = random(opts.seed ?? 1);
	const index = new Map(nodes.map((node, i) => [node.id, i]));

	// One direction per group (spread on a sphere), so folders form clusters.
	const groups = [...new Set(nodes.map((x) => x.group))].sort();
	const golden = Math.PI * (3 - Math.sqrt(5));
	const groupDir = new Map(
		groups.map((g, i) => {
			const y = groups.length === 1 ? 0 : 1 - (2 * i) / (groups.length - 1);
			const r = Math.sqrt(Math.max(0, 1 - y * y));
			return [g, [Math.cos(golden * i) * r, y, Math.sin(golden * i) * r]] as const;
		})
	);

	const spread = 10 + Math.cbrt(n) * 6;
	for (let i = 0; i < n; i++) {
		const d = groupDir.get(nodes[i].group) ?? [0, 0, 0];
		for (let k = 0; k < 3; k++) pos[i * 3 + k] = d[k] * spread * 0.6 + (rand() - 0.5) * spread;
	}

	const edges: [number, number][] = [];
	for (const l of links) {
		const a = index.get(l.source);
		const b = index.get(l.target);
		if (a !== undefined && b !== undefined && a !== b) edges.push([a, b]);
	}

	const vel = new Float32Array(n * 3);
	const steps = opts.iterations ?? stepsFor(n);
	const repulse = 60;
	const springLength = 6;
	const springK = 0.08;
	const groupK = 0.02;
	const centreK = 0.004;
	for (let step = 0; step < steps; step++) {
		const cool = 1 - step / steps; // big moves first, small at the end
		const force = new Float32Array(n * 3);
		for (let a = 0; a < n; a++) {
			for (let b = a + 1; b < n; b++) {
				let dx = pos[a * 3] - pos[b * 3];
				let dy = pos[a * 3 + 1] - pos[b * 3 + 1];
				let dz = pos[a * 3 + 2] - pos[b * 3 + 2];
				let d2 = dx * dx + dy * dy + dz * dz;
				if (d2 < 1e-4) {
					dx = rand() - 0.5;
					dy = rand() - 0.5;
					dz = rand() - 0.5;
					d2 = 1e-2;
				}
				const f = repulse / d2;
				const d = Math.sqrt(d2);
				force[a * 3] += (dx / d) * f;
				force[a * 3 + 1] += (dy / d) * f;
				force[a * 3 + 2] += (dz / d) * f;
				force[b * 3] -= (dx / d) * f;
				force[b * 3 + 1] -= (dy / d) * f;
				force[b * 3 + 2] -= (dz / d) * f;
			}
		}
		for (const [a, b] of edges) {
			const dx = pos[b * 3] - pos[a * 3];
			const dy = pos[b * 3 + 1] - pos[a * 3 + 1];
			const dz = pos[b * 3 + 2] - pos[a * 3 + 2];
			const d = Math.sqrt(dx * dx + dy * dy + dz * dz) || 1e-3;
			const f = springK * (d - springLength);
			force[a * 3] += (dx / d) * f;
			force[a * 3 + 1] += (dy / d) * f;
			force[a * 3 + 2] += (dz / d) * f;
			force[b * 3] -= (dx / d) * f;
			force[b * 3 + 1] -= (dy / d) * f;
			force[b * 3 + 2] -= (dz / d) * f;
		}
		for (let i = 0; i < n; i++) {
			const d = groupDir.get(nodes[i].group) ?? [0, 0, 0];
			for (let k = 0; k < 3; k++) {
				const p = pos[i * 3 + k];
				const f = force[i * 3 + k] + (d[k] * spread * 0.5 - p) * groupK - p * centreK;
				vel[i * 3 + k] = (vel[i * 3 + k] + f) * 0.6;
				const v = Math.max(-3, Math.min(3, vel[i * 3 + k])) * cool;
				pos[i * 3 + k] = p + v;
			}
		}
	}
	return pos;
}

/** The ids linked to `id`. */
export function neighbours(id: number, links: readonly LayoutLink[]): Set<number> {
	const out = new Set<number>();
	for (const l of links) {
		if (l.source === id) out.add(l.target);
		else if (l.target === id) out.add(l.source);
	}
	return out;
}

/** A short key that changes when the notes or their links change (for caching positions). */
export function graphKey(nodes: readonly (LayoutNode & { title: string })[], links: readonly LayoutLink[]): string {
	let h = 2166136261;
	const feed = (s: string) => {
		for (let i = 0; i < s.length; i++) {
			h ^= s.charCodeAt(i);
			h = Math.imul(h, 16777619);
		}
	};
	for (const node of nodes) feed(`${node.id}|${node.group}|${node.title};`);
	for (const l of links) feed(`${l.source}-${l.target};`);
	return (h >>> 0).toString(36) + nodes.length.toString(36);
}
