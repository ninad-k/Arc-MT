/**
 * Desk mode in mock mode (spec-nimbus-desk): seats, signals, the clock, the calendar, plans and a
 * scripted Friday. Uses the mock market's symbols (XAUUSD, US100, EURUSD, ...).
 *
 * Chat: "what's moving", "macro" / "fed", "insiders in NVDA", "desk view on gold", "chart gold",
 * "size a long on gold with a stop at 2380" (a plan), "buy gold with a stop at 2380" (a plan and a
 * preview). A size request without a stop is vetoed.
 */
import { ApiError, type ChatRequest, type ChatResponse, type PreviewDto, type ToolCall } from './api';
import type {
	AgentId,
	Bar,
	CalendarMonth,
	Chart,
	DeskState,
	Finding,
	JournalRow,
	Mark,
	Phase,
	PlanRecord,
	PulseResponse,
	Seat,
	SessionStatus,
	Signal
} from './desk/types';
import { SYMS, mid, round, symInfo } from './mock-market';
import { emitMockEvent, openPreview } from './mock-trading';

const TITLES: Record<AgentId, [string, Seat['group']]> = {
	friday: ['Master', 'master'],
	atlas: ['Macro', 'intelligence'],
	capitol: ['Smart money', 'intelligence'],
	scout: ['Recon', 'intelligence'],
	athena: ['Analyst', 'analysis'],
	chartist: ['Technician', 'analysis'],
	oracle: ['Quant', 'analysis'],
	sentinel: ['Risk officer', 'execution'],
	pilot: ['Execution', 'execution'],
	ledger: ['The book', 'execution']
};

const seats: Seat[] = (Object.keys(TITLES) as AgentId[]).map((id) => ({
	id,
	title: TITLES[id][0],
	group: TITLES[id][1],
	state: 'idle',
	summary: null,
	at_ms: null,
	calls: 0
}));
const signals: Signal[] = [];
const plans: PlanRecord[] = [];
const journal: JournalRow[] = [];
let journalId = 1;
let spent = 1.84;

function log(text: string, kind = 'desk_finding'): void {
	journal.unshift({ id: journalId++, ts: new Date().toISOString(), kind, text });
	journal.length = Math.min(journal.length, 40);
}

function activity(agent: AgentId, state: Seat['state'], summary: string | null): void {
	const seat = seats.find((s) => s.id === agent);
	if (seat) {
		if (state === 'working') seat.calls += 1;
		seat.state = state;
		seat.at_ms = Date.now();
		if (summary) seat.summary = summary;
	}
	emitMockEvent({ type: 'desk_activity', agent, state, summary, at_ms: Date.now() });
}

// ---- Clock and calendar (New York, US daylight saving rule) -------------------------------

function nthSunday(year: number, month: number, n: number): number {
	const first = new Date(Date.UTC(year, month, 1)).getUTCDay();
	return 1 + ((7 - first) % 7) + 7 * (n - 1);
}
function nyOffsetHours(d: Date): number {
	const y = d.getUTCFullYear();
	const start = Date.UTC(y, 2, nthSunday(y, 2, 2), 7);
	const end = Date.UTC(y, 10, nthSunday(y, 10, 1), 6);
	const t = d.getTime();
	return t >= start && t < end ? -4 : -5;
}
const HOLIDAYS = ['2026-11-26', '2026-12-25', '2027-01-01', '2027-01-18'];
const EARLY = ['2026-11-27', '2026-12-24'];
const FOMC = ['2026-12-09', '2027-01-27'];

function ymd(d: Date): string {
	return d.toISOString().slice(0, 10);
}

function session(now = new Date()): SessionStatus {
	const off = nyOffsetHours(now);
	const local = new Date(now.getTime() + off * 3600_000);
	const date = ymd(local);
	const wd = local.getUTCDay();
	const trading = wd !== 0 && wd !== 6 && !HOLIDAYS.includes(date);
	const early = EARLY.includes(date);
	const minutes = local.getUTCHours() * 60 + local.getUTCMinutes();
	const close = early ? 13 * 60 : 16 * 60;
	const bounds: [number, Phase][] = [
		[4 * 60, 'pre_market'],
		[9 * 60 + 30, 'open'],
		[close, 'after_hours'],
		[20 * 60, 'closed']
	];
	let phase: Phase = 'closed';
	if (trading) for (const [m, p] of bounds) if (minutes >= m) phase = p;
	const next = trading ? bounds.find(([m]) => m > minutes) : undefined;
	const midnight = Date.UTC(local.getUTCFullYear(), local.getUTCMonth(), local.getUTCDate()) - off * 3600_000;
	const toNext = next ? (midnight + next[0] * 60_000 - now.getTime()) / 1000 : null;
	const labels: Record<Phase, string> = { pre_market: 'Pre-market', open: 'Open', after_hours: 'After-hours', closed: 'Closed' };
	return {
		phase,
		label: labels[phase],
		ny_time: local.toISOString().slice(11, 16),
		ny_date: date,
		ny_zone: off === -4 ? 'EDT' : 'EST',
		next_change: toNext === null ? null : new Date(now.getTime() + toNext * 1000).toISOString(),
		next_phase: next ? next[1] : null,
		seconds_to_change: toNext === null ? null : Math.round(toNext),
		seconds_to_close: phase === 'open' ? Math.round((midnight + close * 60_000 - now.getTime()) / 1000) : null,
		trading_day: trading,
		early_close: early
	};
}

function thirdFriday(year: number, month: number): string {
	const first = new Date(Date.UTC(year, month - 1, 1)).getUTCDay();
	let day = 1 + ((5 - first + 7) % 7) + 14;
	let d = `${year}-${String(month).padStart(2, '0')}-${String(day).padStart(2, '0')}`;
	while (HOLIDAYS.includes(d)) {
		day -= 1;
		d = `${year}-${String(month).padStart(2, '0')}-${String(day).padStart(2, '0')}`;
	}
	return d;
}

function month(year: number, m: number): CalendarMonth {
	const prefix = `${year}-${String(m).padStart(2, '0')}`;
	const marks: Mark[] = [
		...FOMC.filter((d) => d.startsWith(prefix)).map((date) => ({ date, kind: 'fomc' as const, label: 'FOMC decision', source: 'fed schedule' })),
		{ date: thirdFriday(year, m), kind: 'opex', label: 'Monthly options expiry', source: 'exchange' },
		...HOLIDAYS.filter((d) => d.startsWith(prefix)).map((date) => ({ date, kind: 'holiday' as const, label: 'Exchange holiday', source: 'exchange' })),
		...EARLY.filter((d) => d.startsWith(prefix)).map((date) => ({ date, kind: 'early_close' as const, label: 'Early close (13:00)', source: 'exchange' }))
	];
	marks.sort((a, b) => a.date.localeCompare(b.date));
	return { year, month: m, marks };
}

export function deskState(): DeskState {
	const s = session();
	const [y, m] = s.ny_date.split('-').map(Number);
	const next = m === 12 ? [y + 1, 1] : [y, m + 1];
	const nextFomc = FOMC.find((d) => d >= s.ny_date) ?? null;
	return {
		enabled: true,
		brain: { provider: 'claude', model: 'claude-opus-5-5', available: true, reason: null, metered: true },
		session: s,
		calendar: {
			today: s.ny_date,
			months: [month(y, m), month(next[0], next[1])],
			next_fomc: nextFomc
				? { date: nextFomc, source: 'fed schedule', days: Math.round((Date.parse(nextFomc) - Date.parse(s.ny_date)) / 86_400_000) }
				: null
		},
		seats: seats.map((x) => ({ ...x })),
		signals: [...signals].reverse().slice(0, 40),
		plans: [...plans].sort((a, b) => b.issued_ms - a.issued_ms).slice(0, 10),
		journal: journal.slice(0, 25),
		spend: { month: s.ny_date.slice(0, 7), spent_usd: round(spent, 2), budget_usd: 20, warn: false, blocked: false },
		finnhub: false,
		rules: { risk_percent: 0.5, max_open_positions: 2, no_new_trades_before_close_minutes: 10 },
		kill_switch: false
	};
}

// ---- Market data ----------------------------------------------------------------------------

/** Today's simulated change per symbol, fixed for the session. */
const change = new Map(SYMS.map((s) => [s.symbol, round((Math.random() - 0.5) * 3, 2)]));

export function deskPulse(): PulseResponse {
	const quotes = ['XAUUSD', 'US100', 'EURUSD', 'BTCUSD', 'US500', 'NAS100'].map((symbol) => {
		const info = symInfo(symbol);
		return info
			? { symbol, price: round(mid(symbol), info.digits), change_pct: change.get(symbol) ?? null }
			: { symbol, price: null, change_pct: null };
	});
	return { quotes, at: new Date().toISOString() };
}

function bars(symbol: string, n = 90): Bar[] {
	const info = symInfo(symbol);
	const last = mid(symbol) || 100;
	const vol = (info?.vol ?? 0.0002) * 18;
	const out: Bar[] = [];
	let p = last * (1 - vol * 6);
	const now = Math.floor(Date.now() / 1000);
	for (let i = 0; i < n; i++) {
		const drift = (last - p) / (n - i) + (Math.random() - 0.5) * p * vol;
		const open = p;
		const close = p + drift;
		const high = Math.max(open, close) + Math.random() * p * vol * 0.6;
		const low = Math.min(open, close) - Math.random() * p * vol * 0.6;
		const d = info?.digits ?? 2;
		out.push({ time: now - (n - i) * 3600, open: round(open, d), high: round(high, d), low: round(low, d), close: round(close, d) });
		p = close;
	}
	return out;
}

function symbolIn(text: string): string {
	if (/gold|xau/.test(text)) return 'XAUUSD';
	if (/nasdaq|us100|nas/.test(text)) return 'US100';
	if (/euro|eur/.test(text)) return 'EURUSD';
	if (/bitcoin|btc/.test(text)) return 'BTCUSD';
	const t = text.toUpperCase().match(/\b([A-Z]{2,5})\b/);
	return t && symInfo(t[1]) ? t[1] : 'XAUUSD';
}

// ---- Friday (scripted) ------------------------------------------------------------------------

function finding(agent: AgentId, summary: string, data: Record<string, unknown>, notes: string[] = []): Finding {
	return {
		agent,
		summary,
		data,
		sources: [{ name: agent === 'capitol' || agent === 'athena' ? 'mock finnhub' : 'mock mt5', what: 'simulated data', at: new Date().toISOString() }],
		notes
	};
}

function chartFor(symbol: string, plan: PlanRecord | null, band: boolean): Chart {
	const b = bars(symbol);
	const d = symInfo(symbol)?.digits ?? 2;
	const hi = Math.max(...b.map((x) => x.high));
	const lo = Math.min(...b.map((x) => x.low));
	const r = hi - lo;
	const levels: Chart['levels'] = [
		{ kind: 'supply', price: round(hi - r * 0.06, d), to: round(hi, d), label: 'Supply' },
		{ kind: 'demand', price: round(lo, d), to: round(lo + r * 0.06, d), label: 'Demand' }
	];
	if (plan) {
		levels.push({ kind: 'entry', price: plan.plan.entry, to: null, label: `Entry (${plan.plan.side})` });
		if (plan.plan.stop !== null) levels.push({ kind: 'stop', price: plan.plan.stop, to: null, label: 'Stop' });
		if (plan.plan.target !== null) levels.push({ kind: 'target', price: plan.plan.target, to: null, label: 'Target' });
	}
	const last = b[b.length - 1].close;
	return {
		symbol,
		timeframe: 'H1',
		digits: d,
		bars: b,
		levels,
		band: band ? { low: round(last - r * 0.12, d), high: round(last + r * 0.12, d), bars_ahead: 10, label: 'Oracle 10-bar range (middle 80%)' } : null
	};
}

function makePlan(symbol: string, side: 'buy' | 'sell', stop: number | null, target: number | null): PlanRecord {
	const d = symInfo(symbol)?.digits ?? 2;
	const entry = round(mid(symbol), d);
	const okSide = stop === null ? false : side === 'buy' ? stop < entry : stop > entry;
	const riskAmount = 50.35;
	const distance = stop === null ? 0 : Math.abs(entry - stop);
	const contract = symInfo(symbol)?.contract ?? 1;
	const volume = distance > 0 ? Math.max(0.01, Math.floor((riskAmount / (distance * contract)) * 100) / 100) : 0;
	const reasons: string[] = [];
	if (stop === null) reasons.push('A stop is required on every Desk trade.');
	else if (!okSide) reasons.push(`The stop ${stop} is on the wrong side of the entry ${entry} for a ${side}.`);
	const approved = reasons.length === 0;
	const now = Date.now();
	const record: PlanRecord = {
		id: crypto.randomUUID().replaceAll('-', ''),
		plan: {
			symbol,
			broker_symbol: symbol,
			side,
			entry,
			stop,
			target,
			volume: approved ? volume : 0,
			risk_percent: 0.5,
			risk_amount: approved ? riskAmount : null,
			currency: 'USD',
			sizing_error: null
		},
		verdict: approved
			? {
					decision: 'approved',
					reasons: [`Stop ${stop} is on the right side of ${entry}.`, 'Risk 0.5% is within the Desk cap of 0.5%.', '0 of 2 Desk positions in use.'],
					warnings: target === null ? ['No target set.'] : []
				}
			: { decision: 'vetoed', reasons },
		issued_ms: now,
		expires_ms: now + 120_000,
		used: false
	};
	plans.push(record);
	log(`Sentinel ${approved ? 'approved' : 'vetoed'} plan ${record.id.slice(0, 8)} (${side} ${symbol})`, 'sentinel_verdict');
	return record;
}

export function deskChat(req: ChatRequest): ChatResponse {
	const text = req.message.toLowerCase();
	if (/fail/.test(text)) throw new ApiError('llm_error', 'LLM provider failed: connection refused (mock)', 502);
	const symbol = symbolIn(req.message);
	const findings: Finding[] = [];
	const used: AgentId[] = [];
	let chart: Chart | null = null;
	const newPlans: PlanRecord[] = [];
	let preview: PreviewDto | null = null;
	let reply: string;
	log(`Trader: ${req.message}`, 'utterance');

	const nums = [...req.message.matchAll(/\d+(?:\.\d+)?/g)].map((m) => Number(m[0]));
	const wantsTrade = /\b(buy|sell|long|short|size)\b/.test(text);
	if (wantsTrade) {
		const side = /\b(sell|short)\b/.test(text) ? 'sell' : 'buy';
		const stop = /stop/.test(text) && nums.length > 0 ? nums[0] : null;
		const target = /target|tp/.test(text) && nums.length > 1 ? nums[1] : null;
		const plan = makePlan(symbol, side, stop, target);
		newPlans.push(plan);
		used.push('sentinel');
		const v = plan.verdict;
		findings.push(
			finding(
				'sentinel',
				v.decision === 'approved'
					? `Approved: ${side} ${plan.plan.volume} lots ${symbol} at about ${plan.plan.entry}, stop ${stop}, risking 0.5% (50.35 USD).`
					: `Vetoed ${side} ${symbol}: ${v.reasons.join(' ')}`,
				{ plan_id: plan.id, verdict: v }
			)
		);
		chart = chartFor(symbol, plan, false);
		used.push('chartist');
		findings.push(finding('chartist', `Chart switched to ${symbol} H1 with the plan drawn.`, {}));
		if (v.decision === 'approved' && /\b(buy|sell)\b/.test(text)) {
			try {
				preview = openPreview({ symbol, side, sl: stop, tp: target, volume: plan.plan.volume }, req.voice === true);
				plan.used = true;
				used.push('pilot');
				findings.push(finding('pilot', `Preview ready for plan ${plan.id.slice(0, 8)}: ${preview.description}.`, {}));
				log(`Pilot preview for plan ${plan.id.slice(0, 8)}`, 'pilot_preview');
				reply = `Sentinel approved it and Pilot has the preview ready: ${preview.description}. Confirm it when you are happy.`;
			} catch (e) {
				reply = `Sentinel approved the plan, but the gate refused the preview: ${e instanceof Error ? e.message : String(e)}.`;
			}
		} else if (v.decision === 'approved') {
			reply = `Sentinel approved: ${plan.plan.volume} lots ${symbol} risking 0.5% with a stop at ${stop}. Say "buy" to have Pilot prepare it, or use Send to Pilot on the plan card.`;
		} else {
			reply = `Sentinel vetoed that: ${v.reasons.join(' ')}`;
		}
	} else if (/insider|smart money|congress|capitol/.test(text)) {
		used.push('capitol');
		findings.push(finding('capitol', 'No smart-money data: the insider feed is not available.', { insiders: null, congress: null }, ['Insider transactions need a Finnhub key.']));
		reply = 'Capitol has no data on that: the insider feed needs a Finnhub key on the server.';
	} else if (/macro|fed|fomc|rates/.test(text)) {
		used.push('atlas');
		findings.push(finding('atlas', `Next FOMC decision ${FOMC[0]}; US100 ${change.get('US100')}% today.`, {}));
		reply = `Atlas: the next FOMC decision is ${FOMC[0]}, and US100 is ${change.get('US100')}% today. No headline tone without a news key.`;
	} else if (/chart/.test(text)) {
		chart = chartFor(symbol, null, /forecast|band|range/.test(text));
		used.push('chartist');
		findings.push(finding('chartist', `Chart switched to ${symbol} H1 with 2 levels drawn.`, {}));
		reply = `Chartist has ${symbol} on screen with the nearest supply and demand zones.`;
	} else if (/view|analy|outlook|think/.test(text)) {
		used.push('athena', 'oracle', 'chartist');
		chart = chartFor(symbol, null, true);
		findings.push(finding('athena', `${symbol}: nearest supply and demand zones drawn from H1 swings.`, {}, ['No fundamentals (no Finnhub key).']));
		findings.push(finding('oracle', `Over 5 D1 bars, ${symbol} closed higher 54% of the time in 480 past windows.`, {}));
		findings.push(finding('chartist', `Chart switched to ${symbol} H1 with 2 levels drawn and a forecast band.`, {}));
		reply = `Athena marks the zones on ${symbol}, and Oracle says it closed higher over five days 54% of the time historically. That describes the past, not a prediction, and it is not advice.`;
	} else {
		used.push('scout', 'atlas');
		const movers = [...change.entries()].sort((a, b) => Math.abs(b[1]) - Math.abs(a[1])).slice(0, 3);
		const line = movers.map(([s, c]) => `${s} ${c > 0 ? '+' : ''}${c.toFixed(2)}%`).join(', ');
		findings.push(finding('scout', `Biggest moves today: ${line}.`, {}));
		findings.push(finding('atlas', `Next FOMC decision ${FOMC[0]}.`, {}));
		reply = `Scout sees the biggest moves in ${line}. Atlas notes the next FOMC decision on ${FOMC[0]}.`;
	}

	// Light the seats one after another, like the real desk.
	used.forEach((agent, i) => {
		const f = findings.find((x) => x.agent === agent);
		setTimeout(() => activity(agent, 'working', null), i * 220);
		setTimeout(() => {
			activity(agent, 'done', f?.summary ?? null);
			if (f) {
				signals.push({ at_ms: Date.now(), agent, level: agent === 'sentinel' && f.summary.startsWith('Vetoed') ? 'warn' : 'info', text: f.summary });
				log(`${agent}: ${f.summary}`);
			}
		}, i * 220 + 600);
	});
	activity('friday', 'done', reply);
	log(`Friday: ${reply}`, 'assistant_reply');
	spent += 0.07;
	const tool_calls: ToolCall[] = used.map((a) => ({ name: `ask_${a}`, arguments: { symbol }, ok: true, duration_ms: 120 }));
	return {
		session_id: req.session_id ?? crypto.randomUUID(),
		reply,
		provider: 'claude',
		persona: 'friday',
		tool_calls,
		preview,
		notes: [],
		mode: 'desk',
		brain: { provider: 'claude', model: 'claude-opus-5-5' },
		models: [],
		captured: [],
		spend: { month: new Date().toISOString().slice(0, 7), spent_usd: round(spent, 2), budget_usd: 20, warn: false, blocked: false },
		desk: { findings, chart, plans: newPlans }
	};
}

/** `POST /api/desk/plans/{id}/pilot` in mock mode. */
export function deskPilot(id: string): PreviewDto {
	const r = plans.find((p) => p.id === id);
	if (!r) throw new ApiError('not_found', `no Sentinel plan with id ${id}`, 404);
	if (r.verdict.decision !== 'approved') throw new ApiError('gate_rejected', 'Sentinel vetoed this plan, so Pilot cannot act on it', 422);
	if (r.used) throw new ApiError('gate_rejected', 'this Sentinel approval has already been used', 422);
	if (Date.now() >= r.expires_ms) throw new ApiError('gate_rejected', 'this Sentinel approval has expired; ask Sentinel again', 422);
	const p = openPreview({ symbol: r.plan.symbol, side: r.plan.side, sl: r.plan.stop, tp: r.plan.target, volume: r.plan.volume }, false);
	r.used = true;
	activity('pilot', 'working', null);
	setTimeout(() => activity('pilot', 'done', `Preview ready: ${p.description}.`), 300);
	log(`Pilot preview for plan ${id.slice(0, 8)}: ${p.description}`, 'pilot_preview');
	return p;
}
