import { describe, expect, it } from 'vitest';
import {
	DASH,
	bannerHead,
	ciBar,
	directionBadge,
	gateProgress,
	lockChipText,
	lockTone,
	maskAccount,
	rate,
	totpOk,
	weightMoves,
	consensus,
	contributionBar,
	describeMessage,
	dropChips,
	executionRows,
	filterQuery,
	funnelWidth,
	rText,
	relativeTime,
	signed,
	stanceChip,
	stanceClass,
	stanceTone,
	stepStrip,
	stopLabel,
	thresholdLines,
	tierDim,
	tierStopped,
	usageText
} from './format';

describe('stance', () => {
	it('maps to a tone and class', () => {
		expect(stanceTone('bull')).toBe('up');
		expect(stanceTone('bear')).toBe('down');
		expect(stanceTone('neutral')).toBe('mute');
		expect(stanceTone(null)).toBe('mute');
		expect(stanceClass('bear')).toBe('text-down');
	});
	it('renders a chip', () => {
		expect(stanceChip('bull', 50)).toBe('BULL 50');
		expect(stanceChip('neutral', 12)).toBe('NEU 12');
		expect(stanceChip(null, null)).toBe('n/a');
	});
});

describe('signed', () => {
	it('adds a plus sign and keeps two decimals', () => {
		expect(signed(0.178)).toBe('+0.18');
		expect(signed(-0.051)).toBe('-0.05');
		expect(signed(0)).toBe('0.00');
		expect(signed(-0.001)).toBe('0.00');
		expect(signed(null)).toBe(DASH);
		expect(signed(Number.NaN)).toBe(DASH);
	});
	it('formats R', () => {
		expect(rText(1.4)).toBe('+1.4 R');
		expect(rText(null)).toBe(DASH);
	});
});

describe('stopLabel', () => {
	it('prefers the server label', () => {
		expect(stopLabel('council_below_threshold', 'Score +0.18 is below 0.30')).toBe('Score +0.18 is below 0.30');
	});
	it('falls back from the code', () => {
		expect(stopLabel('not_asked')).toMatch(/ask/i);
		expect(stopLabel('sentinel_veto:max_open_positions')).toBe('Sentinel veto: max open positions');
		expect(stopLabel('gate_limit:max_lot')).toBe('Gate limit: max lot');
		expect(stopLabel('trade_failed:10019')).toContain('10019');
		expect(stopLabel('some_new_code')).toBe('some new code');
		expect(stopLabel(null)).toBe(DASH);
	});
});

describe('stepStrip', () => {
	it('marks the stop on the first dot not reached', () => {
		expect(stepStrip(1, 'stopped')).toEqual(['p', 's', 'n', 'n', 'n']);
		expect(stepStrip(0, 'stopped')).toEqual(['s', 'n', 'n', 'n', 'n']);
		expect(stepStrip(4, 'stopped')).toEqual(['p', 'p', 'p', 'p', 's']);
	});
	it('marks waiting and traded', () => {
		expect(stepStrip(2, 'waiting')).toEqual(['p', 'p', 'w', 'n', 'n']);
		expect(stepStrip(5, 'traded')).toEqual(['p', 'p', 'p', 'p', 'p']);
	});
});

describe('contribution bars', () => {
	it('draws right of zero for bull and left for bear', () => {
		expect(contributionBar(0.1)).toEqual({ left: 50, width: 10 });
		const bear = contributionBar(-0.05);
		expect(bear.left).toBeCloseTo(45);
		expect(bear.width).toBeCloseTo(5);
	});
	it('clamps and keeps a sliver for zero', () => {
		expect(contributionBar(5)).toEqual({ left: 50, width: 50 });
		expect(contributionBar(-5)).toEqual({ left: 0, width: 50 });
		expect(contributionBar(0).width).toBeGreaterThan(0);
		expect(contributionBar(Number.NaN).left).toBe(50);
	});
	it('places the threshold lines', () => {
		const [lo, hi] = thresholdLines(0.3);
		expect(lo).toBeCloseTo(20);
		expect(hi).toBeCloseTo(80);
	});
});

describe('dropChips', () => {
	it('sorts by count then code and drops zeros', () => {
		const chips = dropChips({ not_asked: 3, stood: 0, council_below_threshold: 21, members_split: 8, a_code: 3 });
		expect(chips.map((c) => [c.code, c.count])).toEqual([
			['council_below_threshold', 21],
			['members_split', 8],
			['a_code', 3],
			['not_asked', 3]
		]);
		expect(dropChips(undefined)).toEqual([]);
	});
	it('scales funnel widths', () => {
		expect(funnelWidth(21, 42)).toBe(50);
		expect(funnelWidth(0, 42)).toBe(0);
		expect(funnelWidth(1, 1000)).toBe(2);
		expect(funnelWidth(5, 0)).toBe(0);
	});
});

describe('consensus', () => {
	it('sums to 100', () => {
		const c = consensus([
			{ stance: 'bull', confidence: 50, weight: 0.5 },
			{ stance: 'bear', confidence: 40, weight: 0.25 },
			{ stance: 'neutral', confidence: 0, weight: 0.25 }
		]);
		expect(c.bull + c.neutral + c.bear).toBe(100);
		expect(c.bull).toBe(25);
		expect(c.bear).toBe(10);
	});
	it('is all neutral without weights', () => {
		expect(consensus([])).toEqual({ bull: 0, neutral: 100, bear: 0 });
	});
});

describe('relativeTime', () => {
	const now = 1_000_000_000_000;
	it('picks a unit', () => {
		expect(relativeTime(now - 10_000, now)).toBe('just now');
		expect(relativeTime(now - 5 * 60_000, now)).toBe('5 min ago');
		expect(relativeTime(now - 3 * 3600_000, now)).toBe('3 h ago');
		expect(relativeTime(now - 2 * 86_400_000, now)).toBe('2 d ago');
		expect(relativeTime(null, now)).toBe(DASH);
	});
});

describe('tiers', () => {
	it('dims tiers after the stop and marks the stopped one', () => {
		const d = { stage: 'council' as const, status: 'stopped' as const };
		expect(tierStopped('council', d)).toBe(true);
		expect(tierStopped('members', d)).toBe(false);
		expect(tierDim('master', d)).toBe(true);
		expect(tierDim('members', d)).toBe(false);
		expect(tierDim('execution', { stage: 'execution', status: 'traded' })).toBe(false);
		expect(tierDim('master', null)).toBe(false);
		expect(bannerHead(d)).toBe('No trade · stopped at Council');
	});
});

describe('filterQuery', () => {
	it('leaves out empty values and upper-cases the symbol', () => {
		const q = filterQuery({ symbol: ' nvda ', status: '', stage: 'council', stop: '', from_ms: 5, limit: 50 }, { format: 'csv' });
		const p = new URLSearchParams(q);
		expect(p.get('symbol')).toBe('NVDA');
		expect(p.get('stage')).toBe('council');
		expect(p.has('status')).toBe(false);
		expect(p.has('stop')).toBe(false);
		expect(p.get('format')).toBe('csv');
		expect(p.get('from_ms')).toBe('5');
	});
});

describe('prompt rendering', () => {
	it('pretty-prints tool results', () => {
		const v = describeMessage({ role: 'tool_result', call_id: '1', name: 'ask_oracle', content: '{"p_up":0.66}', is_error: false });
		expect(v.label).toBe('tool result: ask_oracle');
		expect(v.body).toContain('"p_up": 0.66');
	});
	it('keeps non-JSON text and shows tool calls', () => {
		expect(describeMessage({ role: 'tool_result', call_id: '1', name: 'x', content: 'plain', is_error: true })).toMatchObject({ body: 'plain', tone: 'down' });
		const a = describeMessage({ role: 'assistant', text: null, tool_calls: [{ id: '1', name: 'ask_scout', arguments: { symbol: 'NVDA' } }] });
		expect(a.body).toBe('-> ask_scout({"symbol":"NVDA"})');
		expect(describeMessage({ role: 'user', text: 'hi' }).body).toBe('hi');
	});
	it('formats usage', () => {
		expect(usageText({ input_tokens: 3912, output_tokens: 611, cost_usd: 0.028 })).toBe('3,912 in · 611 out · $0.028');
		expect(usageText(null)).toBe(DASH);
	});
});

describe('executionRows', () => {
	const base = { stage: 'execution' as const, stop_label: null };
	it('shows an expired preview as a failed confirm', () => {
		const rows = executionRows({ ...base, step: 4, status: 'stopped', stop_code: 'preview_expired' });
		expect(rows.map((r) => r.state)).toEqual(['ok', 'ok', 'fail', 'skip']);
	});
	it('shows a gate stop and a traded decision', () => {
		expect(executionRows({ ...base, step: 4, status: 'stopped', stop_code: 'gate_limit:max_lot' })[1].state).toBe('fail');
		const t = executionRows({ ...base, step: 5, status: 'traded', stop_code: null, tickets: [3317] });
		expect(t.map((r) => r.state)).toEqual(['ok', 'ok', 'ok', 'ok']);
		expect(t[3].note).toBe('ticket ••3317');
	});
	it('skips everything before the preview', () => {
		expect(executionRows({ ...base, step: 2, status: 'waiting', stop_code: 'not_asked' }).every((r) => r.state === 'skip')).toBe(true);
	});
});

describe('learning helpers', () => {
	it('badges a direction', () => {
		expect(directionBadge('tighten').label).toBe('tighten');
		expect(directionBadge('loosen').cls).toContain('warn');
		expect(directionBadge('reweight').cls).toContain('accent');
	});
	it('places the skill interval on the bar', () => {
		const g = ciBar(0.11, 0.04, 0.18)!;
		expect(g.marker).toBeCloseTo(72);
		expect(g.rangeLeft).toBeCloseTo(58);
		expect(g.rangeWidth).toBeCloseTo(28);
		expect(ciBar(0.5, -0.5, 1)!.rangeLeft).toBe(0);
		expect(ciBar(0.5, -0.5, 1)!.marker).toBe(100);
		expect(ciBar(null, 0, 1)).toBeNull();
		expect(ciBar(0.05, null, null)!.hasRange).toBe(false);
	});
	it('formats rates and gates', () => {
		expect(rate(0.584)).toBe('58%');
		expect(rate(null)).toBe(DASH);
		expect(gateProgress(12, 20)).toEqual({ pct: 60, done: false });
		expect(gateProgress(30, 20)).toEqual({ pct: 100, done: true });
		expect(gateProgress(0, 0).done).toBe(true);
	});
	it('masks the account', () => {
		expect(maskAccount(53107904)).toBe('••7904');
		expect(maskAccount('53107904')).toBe('••7904');
		expect(maskAccount(0)).toBe('off');
		expect(maskAccount(null)).toBe('off');
		expect(maskAccount(12)).toBe('••12');
	});
	it('words the lock chip', () => {
		expect(lockChipText({ broker: 'IC Markets', masked: '••7904', state: 'locked' })).toBe('IC Markets ••7904 · locked ✓');
		expect(lockChipText({ broker: 'IC Markets', masked: 'off', state: 'off' })).toBe('IC Markets · lock off');
		expect(lockTone('mismatch')).toBe('down');
		expect(lockTone('unverified')).toBe('warn');
		expect(lockTone('off')).toBe('mute');
	});
	it('lists weight moves, biggest first', () => {
		const m = weightMoves({ a: 0.17, b: 0.17, c: 0.2 }, { a: 0.22, b: 0.14, c: 0.2 });
		expect(m.map((x) => x.agent)).toEqual(['a', 'b']);
		expect(weightMoves({ a: 0.2 }, { a: 0.2 })).toEqual([]);
		expect(totpOk('123456')).toBe(true);
		expect(totpOk('12345')).toBe(false);
	});
});
