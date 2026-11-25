/**
 * Pure helpers for the Architecture page (spec-architecture-page). No DOM, so they are unit tested.
 * Missing data is always shown as an em dash, never as an invented number.
 */
import type {
	DecisionFilter,
	DecisionStatus,
	DecisionSummary,
	Message,
	Stage,
	Stance,
	Step
} from './types';

export const DASH = '—';

export const STAGES: Stage[] = ['signal', 'members', 'council', 'master', 'execution'];

export const STAGE_LABEL: Record<Stage, string> = {
	signal: 'Signals',
	members: 'Members',
	council: 'Council',
	master: 'Master',
	execution: 'Execution'
};

export type Tone = 'up' | 'down' | 'mute';

/** Stance to a colour tone. */
export function stanceTone(stance: Stance | null | undefined): Tone {
	return stance === 'bull' ? 'up' : stance === 'bear' ? 'down' : 'mute';
}

/** Tailwind text class for a stance (literal names so Tailwind sees them). */
export function stanceClass(stance: Stance | null | undefined): string {
	return stance === 'bull' ? 'text-up' : stance === 'bear' ? 'text-down' : 'text-mute';
}

/** Tailwind background class for a stance. */
export function stanceBg(stance: Stance | null | undefined): string {
	return stance === 'bull' ? 'bg-up' : stance === 'bear' ? 'bg-down' : 'bg-mute';
}

/** `BULL 50`, `NEU 12`, or `n/a` when there is no opinion. */
export function stanceChip(stance: Stance | null | undefined, confidence: number | null | undefined): string {
	if (!stance || typeof confidence !== 'number') return 'n/a';
	const name = stance === 'bull' ? 'BULL' : stance === 'bear' ? 'BEAR' : 'NEU';
	return `${name} ${Math.round(confidence)}`;
}

/** A signed score such as `+0.18` or `-0.05`; "—" for null. */
export function signed(v: number | null | undefined, digits = 2): string {
	if (typeof v !== 'number' || !Number.isFinite(v)) return DASH;
	const s = v.toFixed(digits);
	if (Number(s) === 0) return (0).toFixed(digits);
	return v > 0 ? `+${s}` : s;
}

/** `+1.4 R` style text for the table; "—" for null. */
export function rText(r: number | null | undefined): string {
	return typeof r === 'number' && Number.isFinite(r) ? `${signed(r, 1)} R` : DASH;
}

export function rClass(r: number | null | undefined): string {
	if (typeof r !== 'number' || !Number.isFinite(r) || r === 0) return 'text-faint';
	return r > 0 ? 'text-up' : 'text-down';
}

const STOP_TEXT: Record<string, string> = {
	market_closed: 'The market is closed',
	pre_close_cutoff: 'Too close to the US close for new trades',
	event_blackout: 'A high-impact event is too near',
	budget_exhausted: 'The Desk budget is used up',
	no_data: 'No data for this symbol',
	members_split: 'Members split: too few took a side',
	stale_data: 'Member data is stale',
	council_below_threshold: 'Council score below the threshold',
	not_asked: 'Council passed. Waiting for you to ask Friday',
	friday_stood_aside: 'Friday stood aside',
	plan_invalid: "Friday's plan was not valid",
	kill_switch: 'The kill switch is engaged',
	account_mismatch: 'MT5 is not on the locked master account',
	intent_mismatch: 'The order did not match the confirmed intent',
	preview_expired: 'The preview expired before you confirmed',
	user_declined: 'You declined the preview',
	outcome_unknown: 'Outcome unknown: check positions'
};

/** Plain-English reason: the server's label first, then a fallback from the code. */
export function stopLabel(code: string | null | undefined, label?: string | null): string {
	if (label) return label;
	if (!code) return DASH;
	if (STOP_TEXT[code]) return STOP_TEXT[code];
	const [head, rest] = [code.split(':')[0], code.slice(code.indexOf(':') + 1)];
	if (code.includes(':')) {
		const r = rest.replace(/_/g, ' ');
		if (head === 'sentinel_veto') return `Sentinel veto: ${r}`;
		if (head === 'gate_limit') return `Gate limit: ${r}`;
		if (head === 'trade_failed') return `The broker refused the trade (${rest})`;
	}
	return code.replace(/[:_]/g, ' ');
}

export type DotState = 'p' | 's' | 'w' | 'n';

/**
 * The five dots of the stage strip (members, council, plan, preview, traded). A dot is passed when
 * `step` reached it; the next one is red when the decision stopped, amber while it waits.
 */
export function stepStrip(step: number, status: DecisionStatus): DotState[] {
	const out: DotState[] = [];
	let marked = false;
	for (let i = 1; i <= 5; i++) {
		if (step >= i) out.push('p');
		else if (!marked && status === 'stopped') {
			out.push('s');
			marked = true;
		} else if (!marked && (status === 'waiting' || status === 'open')) {
			out.push('w');
			marked = true;
		} else out.push('n');
	}
	// A stop before the members ran (step 0) marks the first dot.
	return out;
}

export const DOT_NAMES = ['Members', 'Council', 'Plan', 'Preview', 'Traded'];

export function stripLabel(step: number, status: DecisionStatus): string {
	const dots = stepStrip(step, status);
	return dots.map((d, i) => `${DOT_NAMES[i]}: ${d === 'p' ? 'passed' : d === 's' ? 'stopped' : d === 'w' ? 'waiting' : 'not reached'}`).join(', ');
}

export interface BarGeometry {
	/** Percent of the track, 0..100. */
	left: number;
	width: number;
}

/** A bar around the zero line of a track. `scale` is the value that fills one half. */
export function contributionBar(value: number, scale = 0.5): BarGeometry {
	const v = Number.isFinite(value) ? value : 0;
	const w = Math.min(Math.abs(v) / scale, 1) * 50;
	return { left: v >= 0 ? 50 : 50 - w, width: Math.max(w, 0.6) };
}

/** Where the two dashed threshold lines sit on the same track (percent). */
export function thresholdLines(threshold: number, scale = 0.5): [number, number] {
	const t = Math.min(Math.abs(threshold) / scale, 1) * 50;
	return [50 - t, 50 + t];
}

export interface DropChip {
	code: string;
	label: string;
	count: number;
}

/** The funnel's drop chips for one step, biggest first (ties by code). Zero counts are left out. */
export function dropChips(dropped: Record<string, number> | null | undefined): DropChip[] {
	return Object.entries(dropped ?? {})
		.filter(([, n]) => n > 0)
		.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
		.map(([code, count]) => ({ code, label: stopLabel(code), count }));
}

/** A funnel bar width as percent of the first stage (at least a sliver when non-zero). */
export function funnelWidth(count: number, first: number): number {
	if (!(first > 0) || !(count > 0)) return 0;
	return Math.max(2, Math.min(100, (count / first) * 100));
}

export interface Consensus {
	bull: number;
	neutral: number;
	bear: number;
}

/**
 * Bull, neutral and bear shares in percent (summing to 100): each member counts its weight, and
 * the part of it that is conviction goes to its side, the rest to neutral.
 */
export function consensus(items: { stance: Stance; confidence: number; weight: number }[]): Consensus {
	let bull = 0;
	let bear = 0;
	let total = 0;
	for (const it of items) {
		const w = Math.max(0, it.weight);
		const c = Math.min(Math.max(it.confidence, 0), 100) / 100;
		total += w;
		if (it.stance === 'bull') bull += w * c;
		else if (it.stance === 'bear') bear += w * c;
	}
	if (total <= 0) return { bull: 0, neutral: 100, bear: 0 };
	const b = Math.round((bull / total) * 100);
	const r = Math.round((bear / total) * 100);
	return { bull: b, neutral: Math.max(0, 100 - b - r), bear: r };
}

/** `5 min ago`, `2 h ago`, `3 d ago`. */
export function relativeTime(ms: number | null | undefined, now = Date.now()): string {
	if (typeof ms !== 'number' || !Number.isFinite(ms)) return DASH;
	const s = Math.round((now - ms) / 1000);
	if (s < 0) return 'just now';
	if (s < 45) return 'just now';
	const m = Math.round(s / 60);
	if (m < 60) return `${m} min ago`;
	const h = Math.round(m / 60);
	if (h < 24) return `${h} h ago`;
	return `${Math.round(h / 24)} d ago`;
}

/** `HH:MM` in the viewer's time zone. */
export function clock(ms: number | string | null | undefined): string {
	if (ms === null || ms === undefined) return DASH;
	const d = new Date(ms);
	if (Number.isNaN(d.getTime())) return DASH;
	return d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
}

/** The table's "Result / why not" text and its colour class. */
export function resultText(d: DecisionSummary): { text: string; cls: string } {
	switch (d.status) {
		case 'traded':
			return { text: 'Executed', cls: 'text-up' };
		case 'waiting':
			return { text: stopLabel(d.stop_code, d.stop_label), cls: 'text-warn' };
		case 'open':
			return { text: 'In progress', cls: 'text-mute' };
		default:
			return { text: stopLabel(d.stop_code, d.stop_label), cls: 'text-down' };
	}
}

/** The stage a decision ended in, as a label, e.g. "Council". */
export function stoppedAt(d: Pick<DecisionSummary, 'stage'>): string {
	return STAGE_LABEL[d.stage];
}

/** Banner headline: "Stopped at Council: ..." / "Executed" / "Waiting for Friday". */
export function bannerHead(d: Pick<DecisionSummary, 'status' | 'stage'>): string {
	if (d.status === 'traded') return 'Executed';
	if (d.status === 'waiting') return `Waiting at ${STAGE_LABEL[d.stage]}`;
	if (d.status === 'open') return 'In progress';
	return `No trade · stopped at ${STAGE_LABEL[d.stage]}`;
}

/** True when the tier has not been reached by a decision that ended in `stage`. */
export function tierDim(tier: Stage, d: Pick<DecisionSummary, 'stage' | 'status'> | null): boolean {
	if (!d || d.status === 'traded') return false;
	return STAGES.indexOf(tier) > STAGES.indexOf(d.stage);
}

export function tierStopped(tier: Stage, d: Pick<DecisionSummary, 'stage' | 'status'> | null): boolean {
	return !!d && d.status === 'stopped' && d.stage === tier;
}

/** Query string for the list, the export links and the funnel. Empty values are left out. */
export function filterQuery(f: DecisionFilter, extra: Record<string, string> = {}): string {
	const q = new URLSearchParams();
	if (f.from_ms != null) q.set('from_ms', String(Math.round(f.from_ms)));
	if (f.to_ms != null) q.set('to_ms', String(Math.round(f.to_ms)));
	if (f.symbol?.trim()) q.set('symbol', f.symbol.trim().toUpperCase());
	if (f.status) q.set('status', f.status);
	if (f.stage) q.set('stage', f.stage);
	if (f.stop) q.set('stop', f.stop);
	if (f.limit != null) q.set('limit', String(f.limit));
	for (const [k, v] of Object.entries(extra)) q.set(k, v);
	return q.toString();
}

export type RangeId = '24h' | '7d' | '30d';
export const RANGE_MS: Record<RangeId, number> = {
	'24h': 24 * 3600_000,
	'7d': 7 * 24 * 3600_000,
	'30d': 30 * 24 * 3600_000
};

export interface MessageView {
	label: string;
	tone: 'accent' | 'violet' | 'warn' | 'down' | 'mute';
	body: string;
}

function pretty(text: string): string {
	try {
		return JSON.stringify(JSON.parse(text), null, 2);
	} catch {
		return text;
	}
}

/** One prompt message as readable text, for the "Exact prompt & inputs" section. */
export function describeMessage(m: Message): MessageView {
	if (m.role === 'user') return { label: 'user', tone: 'accent', body: m.text };
	if (m.role === 'assistant') {
		const calls = m.tool_calls.map((c) => `-> ${c.name}(${JSON.stringify(c.arguments)})`).join('\n');
		return { label: 'assistant', tone: 'violet', body: [m.text, calls].filter(Boolean).join('\n') || DASH };
	}
	return {
		label: `tool result: ${m.name}`,
		tone: m.is_error ? 'down' : 'mute',
		body: pretty(m.content)
	};
}

/** `3,912 in · 611 out · $0.028`. */
export function usageText(u: { input_tokens: number; output_tokens: number; cost_usd: number } | null | undefined): string {
	if (!u) return DASH;
	return `${u.input_tokens.toLocaleString('en-US')} in · ${u.output_tokens.toLocaleString('en-US')} out · $${u.cost_usd.toFixed(3)}`;
}

/** Is this step a stage the decision has passed? */
export const reached = (step: Step | number, n: number) => step >= n;

export interface ExecRow {
	id: string;
	label: string;
	state: 'ok' | 'fail' | 'skip';
	note: string;
}

/** The five execution checks of a decision, from where it stopped. */
export function executionRows(
	d: Pick<DecisionSummary, 'step' | 'status' | 'stop_code' | 'stop_label' | 'stage'> & { tickets?: number[] }
): ExecRow[] {
	const code = d.stop_code ?? '';
	const hasPreview = d.step >= 4;
	const gateFail = code.startsWith('gate_limit') || ['kill_switch', 'account_mismatch', 'intent_mismatch'].includes(code);
	const declined = code === 'user_declined' || code === 'preview_expired';
	const traded = d.status === 'traded';
	const brokerFail = code.startsWith('trade_failed') || code === 'outcome_unknown';
	const why = stopLabel(d.stop_code, d.stop_label);
	return [
		{ id: 'pilot', label: 'Pilot preview, Sentinel re-checked', state: hasPreview ? 'ok' : 'skip', note: hasPreview ? 'ready' : 'not reached' },
		{ id: 'gate', label: 'Gate: limits, kill switch, account lock', state: gateFail ? 'fail' : hasPreview ? 'ok' : 'skip', note: gateFail ? why : hasPreview ? 'passed' : 'not reached' },
		{ id: 'confirm', label: 'Your confirm (15 s token)', state: traded || brokerFail ? 'ok' : declined ? 'fail' : 'skip', note: declined ? why : traded || brokerFail ? 'confirmed' : 'not reached' },
		{
			id: 'broker',
			label: 'Broker result',
			state: traded ? 'ok' : brokerFail ? 'fail' : 'skip',
			note: traded ? (d.tickets?.length ? `ticket ••${String(d.tickets[0]).slice(-4)}` : 'filled') : brokerFail ? why : 'not reached'
		}
	];
}

// ---- Learning loop and account lock -------------------------------------------------------------

export interface DirectionBadge {
	label: string;
	/** Tailwind classes for the pill. */
	cls: string;
}

/** The badge for a suggestion: tighten is safe (up), loosen needs care (warn), reweight is neutral. */
export function directionBadge(direction: 'reweight' | 'tighten' | 'loosen'): DirectionBadge {
	if (direction === 'tighten') return { label: 'tighten', cls: 'border-up text-up' };
	if (direction === 'loosen') return { label: 'loosen', cls: 'border-warn text-warn' };
	return { label: 'reweight', cls: 'border-accent text-accent' };
}

export interface CiGeometry {
	/** Percent of the track (0..100). */
	marker: number;
	rangeLeft: number;
	rangeWidth: number;
	/** Whether the interval was given. */
	hasRange: boolean;
}

/** The skill CI bar: zero at the middle, `scale` fills one half. Missing values give a null result. */
export function ciBar(
	skill: number | null | undefined,
	low: number | null | undefined,
	high: number | null | undefined,
	scale = 0.25
): CiGeometry | null {
	if (typeof skill !== 'number' || !Number.isFinite(skill)) return null;
	const x = (v: number) => Math.min(100, Math.max(0, 50 + (v / scale) * 50));
	const hasRange = typeof low === 'number' && typeof high === 'number' && Number.isFinite(low) && Number.isFinite(high);
	const l = hasRange ? x(Math.min(low, high)) : x(skill);
	const h = hasRange ? x(Math.max(low, high)) : x(skill);
	return { marker: x(skill), rangeLeft: l, rangeWidth: Math.max(h - l, 0), hasRange };
}

/** A 0..1 rate as a whole percent, or "—". */
export function rate(v: number | null | undefined): string {
	return typeof v === 'number' && Number.isFinite(v) ? `${Math.round(v * 100)}%` : DASH;
}

/** Progress toward a minimum: percent (capped) and whether it is met. */
export function gateProgress(have: number, need: number): { pct: number; done: boolean } {
	if (!(need > 0)) return { pct: 100, done: true };
	return { pct: Math.min(100, Math.max(0, (have / need) * 100)), done: have >= need };
}

/** An account number hidden but for its last four digits: `••7904`. 0 means the lock is off. */
export function maskAccount(n: number | string | null | undefined): string {
	const s = String(n ?? '').replace(/\D/g, '');
	if (!s || /^0+$/.test(s)) return 'off';
	return `••${s.slice(-4)}`;
}

export type LockTone = 'up' | 'down' | 'warn' | 'mute';

export function lockTone(state: 'locked' | 'mismatch' | 'unverified' | 'off'): LockTone {
	return state === 'locked' ? 'up' : state === 'mismatch' ? 'down' : state === 'unverified' ? 'warn' : 'mute';
}

/** The chip text: `IC Markets ••7904 · locked ✓`. */
export function lockChipText(v: { broker: string; masked: string; state: 'locked' | 'mismatch' | 'unverified' | 'off' }): string {
	const tail = v.state === 'locked' ? 'locked ✓' : v.state === 'mismatch' ? 'mismatch' : v.state === 'unverified' ? 'unverified' : 'lock off';
	return v.state === 'off' ? `${v.broker} · ${tail}` : `${v.broker} ${v.masked} · ${tail}`;
}

/** What a weights item changes, largest move first: `Oracle 0.17 → 0.22`. */
export function weightMoves(a: object, b: object): { agent: string; old: number; new: number }[] {
	const oldW = a as Record<string, number>;
	const newW = b as Record<string, number>;
	return Object.keys(newW)
		.filter((k) => Math.abs((newW[k] ?? 0) - (oldW[k] ?? 0)) >= 0.0005)
		.map((k) => ({ agent: k, old: oldW[k] ?? 0, new: newW[k] }))
		.sort((a, b) => Math.abs(b.new - b.old) - Math.abs(a.new - a.old));
}

/** Is a TOTP code well formed (six digits)? */
export const totpOk = (s: string) => /^\d{6}$/.test(s);
