import { describe, expect, it } from 'vitest';
import {
	DASH,
	chartRange,
	countdown,
	direction,
	monthGrid,
	num,
	pct,
	priceToY,
	prob,
	ringPosition,
	secondsLeft
} from './format';

describe('missing data', () => {
	it('is a dash, never a number', () => {
		expect(num(null)).toBe(DASH);
		expect(num(undefined)).toBe(DASH);
		expect(num(Number.NaN)).toBe(DASH);
		expect(pct(null)).toBe(DASH);
		expect(prob(undefined)).toBe(DASH);
		expect(countdown(null)).toBe(DASH);
		expect(countdown(-5)).toBe(DASH);
	});

	it('formats real values', () => {
		expect(num(1.2345, 2)).toBe('1.23');
		expect(pct(1.5)).toBe('+1.50%');
		expect(pct(-0.42)).toBe('-0.42%');
		expect(pct(0)).toBe('0.00%');
		expect(prob(0.634)).toBe('63%');
	});
});

describe('countdown', () => {
	it('picks the two largest units', () => {
		expect(countdown(45)).toBe('45s');
		expect(countdown(125)).toBe('2m 05s');
		expect(countdown(3 * 3600 + 5 * 60)).toBe('3h 05m');
		expect(countdown(2 * 86_400 + 3600)).toBe('2d 1h');
	});
});

describe('direction', () => {
	it('treats tiny moves as flat', () => {
		expect(direction(0.001)).toBe('flat');
		expect(direction(0.2)).toBe('up');
		expect(direction(-0.2)).toBe('down');
		expect(direction(null)).toBe('flat');
	});
});

describe('ring', () => {
	it('starts at the top and goes clockwise', () => {
		expect(ringPosition(0, 4, 40)).toEqual({ x: 50, y: 10 });
		expect(ringPosition(1, 4, 40)).toEqual({ x: 90, y: 50 });
		expect(ringPosition(2, 4, 40)).toEqual({ x: 50, y: 90 });
	});
});

describe('chart range', () => {
	it('covers bars, levels and the band with headroom', () => {
		const bars = [{ time: 0, open: 10, high: 12, low: 9, close: 11 }];
		const r = chartRange(bars, [{ kind: 'stop', price: 8, to: null, label: '' }], {
			low: 7,
			high: 15,
			bars_ahead: 5,
			label: ''
		});
		expect(r).not.toBeNull();
		expect(r!.min).toBeLessThan(7);
		expect(r!.max).toBeGreaterThan(15);
		expect(priceToY(r!.max, r!, 100)).toBe(0);
		expect(priceToY(r!.min, r!, 100)).toBe(100);
	});

	it('is null with nothing to draw', () => {
		expect(chartRange([])).toBeNull();
	});
});

describe('month grid', () => {
	it('lays out November 2026 Monday first with marks', () => {
		const weeks = monthGrid(
			2026,
			11,
			[{ date: '2026-11-20', kind: 'opex', label: 'Opex', source: 'exchange' }],
			'2026-11-24'
		);
		// 1 November 2026 is a Sunday: six blanks before it.
		expect(weeks[0].slice(0, 6).every((d) => d.date === null)).toBe(true);
		expect(weeks[0][6].day).toBe(1);
		expect(weeks[0][6].weekend).toBe(true);
		const all = weeks.flat();
		expect(all.find((d) => d.date === '2026-11-20')!.marks).toHaveLength(1);
		expect(all.find((d) => d.date === '2026-11-24')!.today).toBe(true);
		expect(all.length % 7).toBe(0);
	});
});

describe('secondsLeft', () => {
	it('never goes below zero', () => {
		expect(secondsLeft(10_000, 4_000)).toBe(6);
		expect(secondsLeft(1_000, 4_000)).toBe(0);
	});
});
