/**
 * Pure helpers for the Desk HUD (spec-nimbus-desk). No DOM, so they are unit tested.
 * Missing data is always shown as an em dash, never as an invented number.
 */
import type { Bar, Band, Level, Mark, Phase, SeatState } from './types';

export const DASH = '—';

/** A number with fixed decimals, or "—" for null, undefined or non-finite. */
export function num(v: number | null | undefined, digits = 2): string {
	return typeof v === 'number' && Number.isFinite(v) ? v.toFixed(digits) : DASH;
}

/** A signed percentage (`+1.25%`), or "—". */
export function pct(v: number | null | undefined, digits = 2): string {
	if (typeof v !== 'number' || !Number.isFinite(v)) return DASH;
	return `${v > 0 ? '+' : ''}${v.toFixed(digits)}%`;
}

/** A probability 0..1 as a whole percentage, or "—". */
export function prob(v: number | null | undefined): string {
	return typeof v === 'number' && Number.isFinite(v) ? `${Math.round(v * 100)}%` : DASH;
}

/** `up`, `down` or `flat` for colouring a change. */
export function direction(v: number | null | undefined): 'up' | 'down' | 'flat' {
	if (typeof v !== 'number' || !Number.isFinite(v) || Math.abs(v) < 0.005) return 'flat';
	return v > 0 ? 'up' : 'down';
}

/** `1h 05m`, `12m 03s`, `45s`; "—" for null or negative. */
export function countdown(seconds: number | null | undefined): string {
	if (typeof seconds !== 'number' || !Number.isFinite(seconds) || seconds < 0) return DASH;
	const s = Math.floor(seconds);
	const d = Math.floor(s / 86_400);
	const h = Math.floor((s % 86_400) / 3600);
	const m = Math.floor((s % 3600) / 60);
	const sec = s % 60;
	const pad = (n: number) => String(n).padStart(2, '0');
	if (d > 0) return `${d}d ${h}h`;
	if (h > 0) return `${h}h ${pad(m)}m`;
	if (m > 0) return `${m}m ${pad(sec)}s`;
	return `${sec}s`;
}

export function phaseTone(phase: Phase | null | undefined): 'up' | 'warn' | 'mute' {
	if (phase === 'open') return 'up';
	if (phase === 'pre_market' || phase === 'after_hours') return 'warn';
	return 'mute';
}

export function seatTone(state: SeatState): 'accent' | 'up' | 'down' | 'mute' | 'faint' {
	switch (state) {
		case 'working':
			return 'accent';
		case 'done':
			return 'up';
		case 'error':
			return 'down';
		case 'offline':
			return 'faint';
		default:
			return 'mute';
	}
}

/**
 * Where seat `i` of `n` sits on a ring, as percentages of the box (0..100), starting at the top
 * and going clockwise. `radius` is a percentage of the half-width.
 */
export function ringPosition(i: number, n: number, radius = 42): { x: number; y: number } {
	const angle = -Math.PI / 2 + (2 * Math.PI * i) / Math.max(1, n);
	return {
		x: Math.round((50 + radius * Math.cos(angle)) * 100) / 100,
		y: Math.round((50 + radius * Math.sin(angle)) * 100) / 100
	};
}

/** The price range a chart must show: every bar, every level and the band, with 4% headroom. */
export function chartRange(
	bars: Bar[],
	levels: Level[] = [],
	band: Band | null = null
): { min: number; max: number } | null {
	const values: number[] = [];
	for (const b of bars) values.push(b.high, b.low);
	for (const l of levels) {
		values.push(l.price);
		if (l.to !== null) values.push(l.to);
	}
	if (band) values.push(band.low, band.high);
	const finite = values.filter((v) => Number.isFinite(v));
	if (finite.length === 0) return null;
	let min = Math.min(...finite);
	let max = Math.max(...finite);
	if (max === min) {
		const pad = Math.abs(max) * 0.01 || 1;
		min -= pad;
		max += pad;
	}
	const head = (max - min) * 0.04;
	return { min: min - head, max: max + head };
}

/** Map a price to a y coordinate in a box of `height` (top is the maximum). */
export function priceToY(price: number, range: { min: number; max: number }, height: number): number {
	return ((range.max - price) / (range.max - range.min)) * height;
}

export interface CalendarDay {
	/** `YYYY-MM-DD`, or null for a blank cell before the 1st / after the last day. */
	date: string | null;
	day: number | null;
	marks: Mark[];
	today: boolean;
	weekend: boolean;
}

/** A month as weeks of seven days, Monday first. */
export function monthGrid(year: number, month: number, marks: Mark[], today: string): CalendarDay[][] {
	const first = new Date(Date.UTC(year, month - 1, 1));
	const days = new Date(Date.UTC(year, month, 0)).getUTCDate();
	const lead = (first.getUTCDay() + 6) % 7;
	const cells: CalendarDay[] = [];
	for (let i = 0; i < lead; i++) cells.push({ date: null, day: null, marks: [], today: false, weekend: false });
	for (let d = 1; d <= days; d++) {
		const date = `${year}-${String(month).padStart(2, '0')}-${String(d).padStart(2, '0')}`;
		const wd = (lead + d - 1) % 7;
		cells.push({
			date,
			day: d,
			marks: marks.filter((m) => m.date === date),
			today: date === today,
			weekend: wd >= 5
		});
	}
	while (cells.length % 7 !== 0) cells.push({ date: null, day: null, marks: [], today: false, weekend: false });
	const weeks: CalendarDay[][] = [];
	for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7));
	return weeks;
}

export const MONTHS = [
	'January',
	'February',
	'March',
	'April',
	'May',
	'June',
	'July',
	'August',
	'September',
	'October',
	'November',
	'December'
];

/** Seconds left before `expiresMs`, never negative. */
export function secondsLeft(expiresMs: number, now = Date.now()): number {
	return Math.max(0, Math.floor((expiresMs - now) / 1000));
}
