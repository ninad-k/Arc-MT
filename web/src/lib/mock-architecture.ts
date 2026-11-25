/**
 * Architecture page in mock mode (spec-architecture-page): a few realistic decisions (stopped at the
 * council, traded, Friday stood aside, a Sentinel veto, an expired preview, split members, and one
 * waiting for "Ask Friday"), the funnel computed from them, reviews and a scan that adds a decision.
 * Routed from mock.ts for every `/api/architecture/*` endpoint.
 */
import { ApiError, type PreviewDto } from './api';
import { deskState, session as deskSession } from './mock-desk';
import { openPreview } from './mock-trading';
import { maskAccount } from './architecture/format';
import type {
	ApplyBody,
	ArchitectureMap,
	AskResponse,
	CasePoint,
	CouncilRecord,
	DecisionDetail,
	DecisionSummary,
	ConfigVersion,
	Funnel,
	Guards,
	LearningResponse,
	LearningReview,
	LockBody,
	LockView,
	ReviewItem,
	VersionList,
	Weights,
	LiveConfig,
	MasterRecord,
	MemberId,
	OpinionRecord,
	Review,
	ReviewInput,
	ScanResponse,
	Stance
} from './architecture/types';

const WEIGHTS = { atlas: 0.16, capitol: 0.15, scout: 0.2, athena: 0.15, chartist: 0.17, oracle: 0.17 };
const GUARDS: Guards = {
	risk_percent: 0.5,
	max_open_positions: 2,
	no_new_trades_before_close_minutes: 10,
	min_reward_risk: 1.5,
	min_council_score: 0.3,
	min_non_neutral: 3
};
let live: LiveConfig = { version: 7, weights: { ...WEIGHTS }, guards: { ...GUARDS } };
const DEFAULTS: LiveConfig = {
	version: null,
	weights: { atlas: 0.1667, capitol: 0.1667, scout: 0.1667, athena: 0.1667, chartist: 0.1667, oracle: 0.1665 },
	guards: GUARDS
};

const TITLES: Record<MemberId, [string, string]> = {
	atlas: ['Macro', 'intelligence'],
	capitol: ['Smart money', 'intelligence'],
	scout: ['Recon', 'intelligence'],
	athena: ['Analyst', 'analysis'],
	chartist: ['Technician', 'analysis'],
	oracle: ['Quant', 'analysis']
};
const MEMBER_IDS = Object.keys(TITLES) as MemberId[];

const SOURCES: Record<MemberId, { name: string; what: string }> = {
	atlas: { name: 'Finnhub news · calendar', what: 'headline tone and index moves' },
	capitol: { name: 'Finnhub insiders · congress', what: 'insider open-market trades' },
	scout: { name: 'Bars (IEX / MT5)', what: 'today move and relative volume' },
	athena: { name: 'Finnhub fundamentals', what: 'supply and demand zones, valuation' },
	chartist: { name: 'Bars (IEX / MT5)', what: 'nearest levels above and below' },
	oracle: { name: 'Empirical returns', what: 'P(up) over the horizon' }
};

type OpSpec = [Stance, number, string];
type Ops = Partial<Record<MemberId, OpSpec>>;

const round = (v: number, d = 3) => Math.round(v * 10 ** d) / 10 ** d;

function opinions(ops: Ops, at: string, outcome: string | null = null): OpinionRecord[] {
	return MEMBER_IDS.map((agent) => {
		const [stance, confidence, ev] = ops[agent] ?? ['neutral', 0, 'No usable data for this symbol'];
		const sign = stance === 'bull' ? 1 : stance === 'bear' ? -1 : 0;
		const weight = WEIGHTS[agent];
		return {
			agent,
			stance,
			confidence,
			weight,
			contribution: round(sign * (confidence / 100) * weight),
			horizon: '4h',
			evidence: [ev],
			sources: [{ ...SOURCES[agent], at }],
			data: agent === 'oracle' ? { p_up: round(0.5 + (sign * confidence) / 200, 2), samples: 480 } : null,
			mapper: 'v1',
			outcome: stance === 'neutral' ? null : outcome,
			move_atr: stance === 'neutral' || !outcome ? null : outcome === 'hit' ? 0.6 : -0.4
		};
	});
}

function council(ops: OpinionRecord[], veto: [string, string] | null = null): CouncilRecord {
	const score = round(ops.reduce((s, o) => s + o.contribution, 0));
	const nonNeutral = ops.filter((o) => o.stance !== 'neutral').length;
	const point = (o: OpinionRecord): CasePoint => ({ agent: o.agent, value: o.contribution, evidence: o.evidence[0] });
	const bullPts = ops.filter((o) => o.stance === 'bull').map(point);
	const bearPts = ops.filter((o) => o.stance === 'bear').map(point);
	const passedScore = Math.abs(score) >= GUARDS.min_council_score && nonNeutral >= GUARDS.min_non_neutral;
	const guards = [
		{ rule: 'Stop required', reason: 'Friday must give a stop' },
		{ rule: `Risk ≤ ${GUARDS.risk_percent}% of equity`, reason: 'Sized from the stop distance' },
		{ rule: `Open Desk positions < ${GUARDS.max_open_positions}`, reason: veto ? veto[1] : '1 open' },
		{ rule: `Not within ${GUARDS.no_new_trades_before_close_minutes} min of the US close`, reason: 'Enough time left' },
		{ rule: `Reward:risk ≥ ${GUARDS.min_reward_risk}`, reason: 'Checked on the plan' }
	];
	const risk = veto ? [{ rule: veto[0].split(':')[1] ?? veto[0], reason: veto[1] }] : guards;
	let stop: [string, string] | null = null;
	if (veto) stop = veto;
	else if (nonNeutral < GUARDS.min_non_neutral)
		stop = ['members_split', `${nonNeutral} of 6 members took a side; ${GUARDS.min_non_neutral} are needed`];
	else if (!passedScore) stop = ['council_below_threshold', `Score ${score.toFixed(2)} is below ${GUARDS.min_council_score.toFixed(2)}`];
	return {
		score,
		threshold: GUARDS.min_council_score,
		non_neutral: nonNeutral,
		contributions: ops.map((o) => ({ agent: o.agent, weight: o.weight, stance: o.stance, confidence: o.confidence, value: o.contribution })),
		bull: { total: round(bullPts.reduce((s, p) => s + p.value, 0)), points: bullPts },
		bear: { total: round(bearPts.reduce((s, p) => s + p.value, 0)), points: bearPts },
		risk,
		passed: passedScore && !veto,
		side: passedScore && !veto ? (score > 0 ? 'bull' : 'bear') : null,
		stop
	};
}

function master(symbol: string, side: 'buy' | 'sell', c: CouncilRecord, reply: string, costIn = 3912): MasterRecord {
	const system =
		'You are Friday, the Desk master. You receive the council packet and the members\' tool results. Propose exactly one plan (side, entry, stop, target, rationale) or stand aside and say why. You have read and propose tools only; Pilot and the human confirm execute.';
	return {
		provider: 'claude',
		model: 'claude-opus-5-5',
		system_hash: '9f3c4e1ab7a1',
		system,
		messages: [
			{ role: 'user', text: `Council passed ${symbol}: score ${c.score.toFixed(2)} (threshold ${c.threshold.toFixed(2)}, weights v7). Propose a plan or stand aside, with stop and target.` },
			{
				role: 'assistant',
				text: null,
				tool_calls: [
					{ id: 'c1', name: 'ask_oracle', arguments: { symbol } },
					{ id: 'c2', name: 'ask_scout', arguments: { symbol } }
				]
			},
			{ role: 'tool_result', call_id: 'c1', name: 'ask_oracle', content: JSON.stringify({ p_up: 0.66, horizon: '4h', samples: 480, range: [119.1, 124.9] }), is_error: false },
			{ role: 'tool_result', call_id: 'c2', name: 'ask_scout', content: JSON.stringify({ direction: side === 'buy' ? 'up' : 'down', change_pct: 2.1, rank: 1, rvol: 1.8 }), is_error: false }
		],
		reply,
		usage: { input_tokens: costIn, output_tokens: 611, cost_usd: 0.028 }
	};
}

// ---- The decisions --------------------------------------------------------------------------

let seq = 0;
const store = new Map<string, DecisionDetail>();
let seeded = false;

interface Spec {
	symbol: string;
	venue: 'mt5' | 'alpaca';
	minsAgo: number;
	trigger: DecisionDetail['trigger'];
	ops: Ops;
	price: number;
	atr: number;
	tail: Partial<DecisionDetail>;
	veto?: [string, string];
	outcome?: string | null;
}

function add(spec: Spec): DecisionDetail {
	const ms = Date.now() - spec.minsAgo * 60_000;
	const at = new Date(ms).toISOString();
	const ops = opinions(spec.ops, at, spec.outcome ?? null);
	const c = council(ops, spec.veto ?? null);
	const id = `dec_${String(++seq).padStart(4, '0')}${spec.symbol.slice(0, 3).toLowerCase()}`;
	const d: DecisionDetail = {
		id,
		created_at: at,
		created_ms: ms,
		symbol: spec.symbol,
		venue: spec.venue,
		trigger: spec.trigger,
		stage: 'council',
		status: 'stopped',
		stop_code: c.stop ? c.stop[0] : null,
		stop_label: null,
		stop_detail: c.stop ? c.stop[1] : null,
		score: c.score,
		threshold: c.threshold,
		council_passed: c.passed,
		side: c.side,
		has_plan: false,
		has_preview: false,
		result_r: null,
		step: 1,
		tags: [],
		session_id: null,
		broker_symbol: spec.symbol,
		price: spec.price,
		atr: spec.atr,
		council: c,
		weights_version: 7,
		master: null,
		plan_id: null,
		plan: null,
		verdict: null,
		preview_id: null,
		tickets: [],
		pnl: null,
		closed_at: null,
		opinions: ops,
		review: null,
		...spec.tail
	};
	store.set(id, d);
	return d;
}

const APPROVED = (reasons: string[]) => ({ decision: 'approved' as const, reasons, warnings: [] });

function seed(): void {
	if (seeded) return;
	seeded = true;
	// EURUSD: three members agree but the score stays under the threshold.
	add({
		symbol: 'EURUSD', venue: 'mt5', minsAgo: 5, price: 1.0843, atr: 0.0006,
		trigger: { kind: 'scan', detail: 'Scout mover +0.42%, rank 2' },
		ops: {
			atlas: ['neutral', 12, 'Macro tone +0.1 (12 headlines); no FOMC within 7 days'],
			scout: ['bull', 50, 'Up 0.42% today, rank 2 of the watchlist'],
			athena: ['bear', 34, 'Supply zone 1.0912, 9 pips above the price'],
			chartist: ['bull', 36, 'Room to 1.0930 is 1.8x the room down to 1.0810'],
			oracle: ['bull', 40, 'P(up, 4h) 0.70 over 480 samples']
		},
		tail: {
			stage: 'council', status: 'stopped', step: 1,
			stop_label: 'Council score +0.18 is below the 0.30 threshold',
			stop_detail: "Athena's supply zone at 1.0912, 9 pips above, outweighed the momentum case."
		}
	});
	// XAUUSD: council passed, Friday proposed, Sentinel approved, the preview expired.
	const xauOps: Ops = {
		atlas: ['bull', 30, 'Risk-off tone: gold bid on falling yields'],
		scout: ['bull', 55, 'Up 0.9% today, volume 1.4x'],
		athena: ['bull', 28, 'Price at a demand zone 2384 to 2388'],
		chartist: ['bull', 40, 'Room to 2412 is 2.1x the room down to 2380'],
		oracle: ['bull', 38, 'P(up, 4h) 0.69 over 480 samples']
	};
	const xau = add({
		symbol: 'XAUUSD', venue: 'mt5', minsAgo: 6, price: 2391.26, atr: 8.4, trigger: { kind: 'scan', detail: 'Market view: bullish 64, setup long' },
		ops: xauOps,
		tail: {}
	});
	xau.master = master('XAUUSD', 'buy', xau.council!, '{"action":"buy","symbol":"XAUUSD","entry":"market","stop":2380,"target":2412,"rationale":"Demand zone and momentum agree."}');
	Object.assign(xau, {
		stage: 'execution', status: 'stopped', step: 4, stop_code: 'preview_expired', stop_label: 'You did not confirm within 15 s, so the preview expired',
		stop_detail: 'No order was sent.', has_plan: true, has_preview: true, plan_id: 'plan_x1', preview_id: 'prev_x1',
		plan: { symbol: 'XAUUSD', broker_symbol: 'XAUUSD', side: 'buy', entry: 2391.26, stop: 2380, target: 2412, volume: 0.1, risk_percent: 0.5, risk_amount: 112.6, currency: 'USD', sizing_error: null },
		verdict: APPROVED(['Stop is set', 'Risk 0.46% of equity', 'Reward:risk 1.8'])
	});
	// AAPL: council passed (bear), Friday stood aside.
	const aapl = add({
		symbol: 'AAPL', venue: 'alpaca', minsAgo: 20, price: 226.4, atr: 3.1, trigger: { kind: 'scan', detail: 'Scout mover -1.8%, rank 3' },
		ops: {
			atlas: ['bear', 22, 'Risk-off tone, indices down'],
			capitol: ['bear', 20, '2 insider sales in 30 days'],
			scout: ['bear', 60, 'Down 1.8% today, volume 1.6x'],
			athena: ['bear', 30, 'Price inside the supply zone 226 to 228'],
			oracle: ['bear', 36, 'P(up, 4h) 0.32 over 480 samples']
		},
		tail: {}
	});
	aapl.master = master('AAPL', 'sell', aapl.council!, 'Standing aside: earnings are due in 1 day, and a short here is a coin flip on the report.');
	Object.assign(aapl, { stage: 'master', status: 'stopped', step: 2, stop_code: 'friday_stood_aside', stop_label: 'Friday stood aside: earnings in 1 day', stop_detail: 'Friday declined to propose a plan.', council_passed: true });
	// US500: only two members took a side.
	add({
		symbol: 'US500', venue: 'mt5', minsAgo: 35, price: 5601, atr: 22, trigger: { kind: 'scan', detail: 'Calendar: ECB speaker in 3 h' },
		ops: { scout: ['bull', 30, 'Up 0.3% today'], oracle: ['bull', 14, 'P(up, 4h) 0.57'] },
		tail: { stage: 'members', status: 'stopped', step: 1, stop_label: 'Members split: 2 of 6 took a side (3 needed)' }
	});
	// NVDA: executed on Alpaca paper, with the full master prompt.
	const nvda = add({
		symbol: 'NVDA', venue: 'alpaca', minsAgo: 75, price: 121.4, atr: 1.4, outcome: 'hit', trigger: { kind: 'scan', detail: 'Scout mover +2.1%, rank 1' },
		ops: {
			atlas: ['neutral', 18, 'Macro tone +0.1 (12 headlines); no FOMC within 7 days'],
			capitol: ['bull', 52, '3 insider buys in 30 days, net +41k shares'],
			scout: ['bull', 61, 'Top mover +2.1%, volume 1.8x the 20-day average'],
			athena: ['bear', 24, 'Forward P/E 38, above the sector; demand zone 119.6 to 120.2 below'],
			chartist: ['bull', 20, 'Band 119.4 to 123.9 over the next 16 bars'],
			oracle: ['bull', 32, 'P(up, 4h) 0.66 over 480 samples']
		},
		tail: {}
	});
	nvda.master = master('NVDA', 'buy', nvda.council!, '{"action":"buy","symbol":"NVDA","entry":"market","stop":119.80,"target":124.60,"rationale":"Momentum and the probability edge agree; the valuation concern is long-horizon."}');
	Object.assign(nvda, {
		stage: 'execution', status: 'traded', step: 5, stop_code: null, stop_detail: null, council_passed: true, has_plan: true, has_preview: true,
		plan_id: 'plan_n1', preview_id: 'prev_n1', tickets: [3317], pnl: 26.9, result_r: 1.4, closed_at: new Date(Date.now() - 20 * 60_000).toISOString(),
		plan: { symbol: 'NVDA', broker_symbol: 'NVDA', side: 'buy', entry: 121.4, stop: 119.8, target: 124.6, volume: 12, risk_percent: 0.19, risk_amount: 19.2, currency: 'USD', sizing_error: null },
		verdict: APPROVED(['Stop is set', 'Risk 0.19% of equity', 'Reward:risk 2.0'])
	});
	// MSFT: Sentinel's open-position guard vetoed it.
	add({
		symbol: 'MSFT', venue: 'alpaca', minsAgo: 90, price: 441.2, atr: 5.2, veto: ['sentinel_veto:max_open_positions', '2 Desk positions are already open (max 2)'],
		trigger: { kind: 'scan', detail: 'Market view: bullish 58' },
		ops: {
			atlas: ['bull', 20, 'Risk-on tone'], scout: ['bull', 50, 'Up 1.1% today'], chartist: ['bull', 40, 'Room above is 1.9x the room below'],
			oracle: ['bull', 40, 'P(up, 4h) 0.70'], athena: ['neutral', 0, 'Price between zones']
		},
		tail: { stage: 'council', status: 'stopped', step: 1, stop_label: 'Sentinel veto: 2 positions are already open (max 2)' }
	});
	// GBPUSD: an older executed short, already reviewed.
	const gbp = add({
		symbol: 'GBPUSD', venue: 'mt5', minsAgo: 190, price: 1.2714, atr: 0.0009, outcome: 'miss', trigger: { kind: 'question', detail: 'Asked in Desk: "short cable?"' },
		ops: {
			atlas: ['bear', 25, 'Risk-off tone'], scout: ['bear', 55, 'Down 0.5% today'], athena: ['bear', 30, 'At a supply zone'],
			chartist: ['bear', 35, 'Room down is 1.7x'], oracle: ['bear', 38, 'P(up, 4h) 0.31']
		},
		tail: {}
	});
	gbp.master = master('GBPUSD', 'sell', gbp.council!, '{"action":"sell","symbol":"GBPUSD","entry":"market","stop":1.2745,"target":1.2660}', 3400);
	Object.assign(gbp, {
		stage: 'execution', status: 'traded', step: 5, stop_code: null, stop_detail: null, council_passed: true, has_plan: true, has_preview: true,
		tickets: [884210002], pnl: -40, result_r: -1, tags: ['trend', 'late entry'],
		plan: { symbol: 'GBPUSD', broker_symbol: 'GBPUSD', side: 'sell', entry: 1.2714, stop: 1.2745, target: 1.266, volume: 0.2, risk_percent: 0.4, risk_amount: 40, currency: 'USD', sizing_error: null },
		verdict: APPROVED(['Stop is set', 'Risk 0.40% of equity']),
		review: { setup: 'trend', mistakes: ['late entry'], regime: 'trend', rating: 2, take_again: false, notes: 'Entered after most of the move.', updated_at: new Date().toISOString() }
	});
	// USDJPY: the council passed; nobody has asked Friday yet.
	add({
		symbol: 'USDJPY', venue: 'mt5', minsAgo: 2, price: 156.234, atr: 0.28, trigger: { kind: 'scan', detail: 'Scout mover +0.6%, rank 1' },
		ops: {
			atlas: ['bull', 24, 'Yield spread widening'], scout: ['bull', 58, 'Up 0.6% today'], athena: ['neutral', 0, 'Between zones'],
			chartist: ['bull', 42, 'Room above is 2.2x'], oracle: ['bull', 44, 'P(up, 4h) 0.72']
		},
		tail: {
			stage: 'master', status: 'waiting', step: 2, stop_code: 'not_asked', stop_label: 'Council passed. Waiting for you to ask Friday',
			stop_detail: 'Asking Friday is a paid Desk turn.'
		}
	});
}

const summary = (d: DecisionDetail): DecisionSummary => ({
	id: d.id, created_at: d.created_at, created_ms: d.created_ms, symbol: d.symbol, venue: d.venue, trigger: d.trigger, stage: d.stage,
	status: d.status, stop_code: d.stop_code, stop_label: d.stop_label, stop_detail: d.stop_detail, score: d.score, threshold: d.threshold,
	council_passed: d.council_passed, side: d.side, has_plan: d.has_plan, has_preview: d.has_preview, result_r: d.result_r, step: d.step,
	tags: [...d.tags]
});

const all = () => {
	seed();
	return [...store.values()].sort((a, b) => b.created_ms - a.created_ms);
};

// ---- Endpoints ------------------------------------------------------------------------------

export function architectureMap(): ArchitectureMap {
	const ds = all();
	const desk = deskState();
	const seat = (id: string) => desk.seats?.find((s) => s.id === id) ?? null;
	return {
		members: MEMBER_IDS.map((agent) => {
			const withOp = ds.filter((d) => d.opinions.some((o) => o.agent === agent));
			const newest = withOp[0];
			const o = newest?.opinions.find((x) => x.agent === agent);
			return {
				agent,
				title: TITLES[agent][0],
				group: TITLES[agent][1],
				weight: WEIGHTS[agent],
				latest: newest && o ? { agent, stance: o.stance, confidence: o.confidence, weight: o.weight, contribution: o.contribution, evidence: o.evidence, symbol: newest.symbol, at: newest.created_at, decision_id: newest.id, opinions: withOp.length } : null,
				seat: seat(agent)
			};
		}),
		council: ['bull_case', 'bear_case', 'risk', 'chair'],
		master: { agent: 'friday', provider: 'claude', model: 'claude-opus-5-5', seat: seat('friday'), spend: desk.spend ?? null },
		execution: (['sentinel', 'pilot', 'gate', 'confirm', 'broker'] as const).map((id) => ({ id, seat: seat(id) })),
		live: structuredClone(live),
		defaults: DEFAULTS,
		session: deskSession(),
		scan_every_minutes: 15,
		kill_switch: false
	};
}

export function architectureDecisions(query: URLSearchParams): { decisions: DecisionSummary[] } {
	const from = Number(query.get('from_ms') ?? 0);
	const to = Number(query.get('to_ms') ?? Number.MAX_SAFE_INTEGER);
	const symbol = query.get('symbol')?.toUpperCase();
	const status = query.get('status');
	const stage = query.get('stage');
	const stop = query.get('stop');
	const limit = Number(query.get('limit') ?? 200);
	const decisions = all()
		.filter((d) => d.created_ms >= from && d.created_ms <= to)
		.filter((d) => !symbol || d.symbol.includes(symbol))
		.filter((d) => !status || d.status === status)
		.filter((d) => !stage || d.stage === stage)
		.filter((d) => !stop || d.stop_code === stop)
		.slice(0, limit)
		.map(summary);
	return { decisions };
}

export function architectureDecision(id: string): DecisionDetail {
	const d = all().find((x) => x.id === id);
	if (!d) throw new ApiError('not_found', `no decision with id ${id}`, 404);
	return structuredClone(d);
}

const FUNNEL_LABELS = ['Signals', 'Members ok', 'Council passed', 'Master proposed', 'Previewed', 'Executed'];

export function architectureFunnel(query: URLSearchParams): Funnel {
	const to = Number(query.get('to_ms') ?? Date.now());
	const from = Number(query.get('from_ms') ?? to - 24 * 3600_000);
	const ds = all().filter((d) => d.created_ms >= from && d.created_ms <= to);
	return {
		from_ms: from,
		to_ms: to,
		steps: FUNNEL_LABELS.map((label, k) => {
			const dropped: Record<string, number> = {};
			for (const d of ds)
				if (d.step === k && d.status !== 'traded' && d.status !== 'open' && d.stop_code) dropped[d.stop_code] = (dropped[d.stop_code] ?? 0) + 1;
			return { step: k as 0 | 1 | 2 | 3 | 4 | 5, label, count: ds.filter((d) => d.step >= k).length, dropped };
		})
	};
}

let scanCount = 0;
export function architectureScan(): ScanResponse {
	const pool = [
		['BTCUSD', 'mt5', 67210, 410],
		['US100', 'mt5', 19842, 96],
		['XAUUSD', 'mt5', 2391.26, 8.4]
	] as const;
	const [symbol, venue, price, atr] = pool[scanCount++ % pool.length];
	const d = add({
		symbol, venue, price, atr, minsAgo: 0, trigger: { kind: 'scan', detail: 'Scan now' },
		ops: {
			scout: ['bull', 56, 'Up 1.2% today'], chartist: ['bull', 40, 'Room above is 2x'], oracle: ['bull', 44, 'P(up, 4h) 0.72'],
			athena: ['neutral', 0, 'Between zones'], atlas: ['bull', 20, 'Risk-on tone']
		},
		tail: { stage: 'master', status: 'waiting', step: 2, stop_code: 'not_asked', stop_label: 'Council passed. Waiting for you to ask Friday', council_passed: true }
	});
	return { ids: [d.id], decisions: [summary(d)] };
}

function validReview(r: ReviewInput): void {
	const bad = (m: string) => {
		throw new ApiError('bad_request', m, 400);
	};
	if (r.rating !== null && (!Number.isInteger(r.rating) || r.rating < 1 || r.rating > 5)) bad('rating must be 1 to 5');
	if (r.mistakes.length > 10) bad('at most 10 mistake tags');
	for (const t of [r.setup, r.regime, ...r.mistakes]) if (t && t.length > 64) bad('a tag is at most 64 characters');
	if (r.notes && r.notes.length > 4000) bad('notes are at most 4000 characters');
}

export function architectureReview(id: string, body: ReviewInput): { review: Review } {
	const d = all().find((x) => x.id === id);
	if (!d) throw new ApiError('not_found', `no decision with id ${id}`, 404);
	validReview(body);
	const review: Review = { ...body, mistakes: [...body.mistakes], updated_at: new Date().toISOString() };
	d.review = review;
	d.tags = [body.setup, ...body.mistakes].filter((t): t is string => !!t);
	return { review: structuredClone(review) };
}

export function architectureAsk(id: string): AskResponse {
	const d = all().find((x) => x.id === id);
	if (!d) throw new ApiError('not_found', `no decision with id ${id}`, 404);
	if (d.status !== 'waiting') throw new ApiError('bad_request', 'only a decision waiting for Friday can be asked', 400);
	const table: Record<string, [number, number, number]> = { USDJPY: [155.6, 157.6, 0.1], BTCUSD: [66000, 70000, 0.01], US100: [19700, 20100, 0.05], XAUUSD: [2380, 2412, 0.1] };
	const info = table[d.symbol] ?? [0, 0, 0.1];
	const preview: PreviewDto = openPreview({ symbol: d.symbol, side: 'buy', sl: info[0], tp: info[1], volume: info[2] }, false);
	const m = master(d.symbol, 'buy', d.council!, `{"action":"buy","symbol":"${d.symbol}","entry":"market","stop":${info[0]},"target":${info[1]},"rationale":"The council agrees and the stop sits under the demand zone."}`);
	Object.assign(d, {
		trigger: { kind: 'ask', detail: d.trigger.detail },
		master: m, stage: 'execution', status: 'open', step: 4, stop_code: null, stop_label: null, stop_detail: null, has_plan: true, has_preview: true,
		plan_id: crypto.randomUUID(), preview_id: preview.id,
		plan: { symbol: d.symbol, broker_symbol: d.symbol, side: 'buy', entry: d.price ?? 0, stop: info[0], target: info[1], volume: info[2], risk_percent: 0.5, risk_amount: null, currency: 'USD', sizing_error: null },
		verdict: APPROVED(['Stop is set', 'Risk within 0.5% of equity'])
	});
	return {
		session_id: crypto.randomUUID(),
		reply: `Plan ready: buy ${d.symbol} with a stop at ${info[0]} and a target at ${info[1]}. Confirm it in the next step.`,
		preview,
		plans: [],
		spend: { month: new Date().toISOString().slice(0, 7), spent_usd: 1.9, budget_usd: 20, warn: false, blocked: false },
		decision: structuredClone(d)
	};
}

// ---- Learning loop -----------------------------------------------------------------------------

const D = 86_400_000;
const versions: ConfigVersion[] = [
	{ version: 7, created_at: new Date(Date.now() - 7 * D).toISOString(), weights: { ...WEIGHTS }, guards: { ...GUARDS }, reason: 'Scout 0.17 → 0.20 (proposal #12)', proposal_id: 'rev_0001:it_0', actor: 'you' },
	{ version: 6, created_at: new Date(Date.now() - 14 * D).toISOString(), weights: { atlas: 0.17, capitol: 0.16, scout: 0.17, athena: 0.17, chartist: 0.17, oracle: 0.16 }, guards: { ...GUARDS }, reason: 'min_council_score 0.25 → 0.30', proposal_id: null, actor: 'you' },
	{ version: 5, created_at: new Date(Date.now() - 21 * D).toISOString(), weights: { ...DEFAULTS.weights }, guards: { ...GUARDS, min_council_score: 0.25 }, reason: 'Initial equal weights', proposal_id: null, actor: 'system' }
];

const PROPOSED: Weights = { atlas: 0.16, capitol: 0.16, scout: 0.2, athena: 0.14, chartist: 0.12, oracle: 0.22 };
let spentUsd = 0.06;
let reviewSeq = 1;
const closed = 9;
let sinceLast = 12;
let lastReviewMs = Date.now() - 3 * D;

function makeReview(): LearningReview {
	const id = `rev_${String(reviewSeq++).padStart(4, '0')}`;
	return {
		id,
		created_at: new Date().toISOString(),
		model: 'claude-sonnet-5-5',
		summary: "Oracle's skill is the clearest edge; Chartist's band capped trend-day targets early. One guard looks worth tightening.",
		items: [
			{
				id: `${id}:w1`, kind: 'weights', param: 'weights', old: { ...live.weights }, new: { ...PROPOSED }, direction: 'reweight',
				rationale: "Oracle's skill is +0.11 (80% CI +0.04 to +0.18) over 84 resolved opinions. Chartist's is -0.03, and its band capped 6 trend-day targets early. Both moves are within the 0.05 cap.",
				evidence_refs: ['dec_0005nvd', 'dec_0007gbp'], confidence: 0.7, locked: null, applied_version: null
			},
			{
				id: `${id}:g1`, kind: 'guard', param: 'no_new_trades_before_close_minutes', old: live.guards.no_new_trades_before_close_minutes, new: 15, direction: 'tighten',
				rationale: 'Entries in the last 15 minutes of the session averaged -0.4 ATR after the stop, while the 10-minute cutoff leaves them open.',
				evidence_refs: ['pre_close_cutoff'], confidence: 0.6, locked: null, applied_version: null
			},
			{
				id: `${id}:g2`, kind: 'guard', param: 'max_open_positions', old: live.guards.max_open_positions, new: 3, direction: 'loosen',
				rationale: '9 decisions were blocked by the position cap, with a counterfactual move of +0.34 ATR. Correlation between the open positions was high on 6 of them.',
				evidence_refs: ['sentinel_veto:max_open_positions'], confidence: 0.4,
				locked: `loosening needs 100 closed decisions, there are ${closed}`, applied_version: null
			}
		],
		rejected: [
			{ item: { param: 'risk_percent', new: 5 }, reason: 'risk_percent 5 is not one of the allowed candidates (0.25, 0.5, 0.75, 1)' },
			{ item: { param: 'weights', new: { oracle: 0.41 } }, reason: 'a weight of 0.41 is outside the 0.08 to 0.33 bounds' }
		],
		cost_usd: 0.031,
		live_version: live.version
	};
}
const reviews: LearningReview[] = [makeReview()];

export function architectureLearning(): LearningResponse {
	const m = (agent: keyof Weights, n: number, taken: number, hit: number, skill: number, lo: number, hi: number) => ({
		agent, n, n_taken: taken, n_counterfactual: n - taken, neutral: 12, flat: 3, hit_rate: hit, hit_rate_taken: hit + 0.02,
		hit_rate_counterfactual: agent === 'capitol' ? null : hit - 0.02, brier: round(0.25 * (1 - skill), 4), skill, skill_low: lo, skill_high: hi
	});
	const canReview = sinceLast >= 20;
	return {
		snapshot: {
			members: [
				m('oracle', 84, 30, 0.58, 0.11, 0.04, 0.18), m('scout', 91, 33, 0.56, 0.07, 0.01, 0.13), m('capitol', 37, 14, 0.57, 0.05, -0.06, 0.14),
				m('atlas', 88, 31, 0.51, 0.01, -0.05, 0.06), m('athena', 79, 28, 0.52, 0.02, -0.04, 0.08), m('chartist', 86, 30, 0.49, -0.03, -0.09, 0.02)
			],
			guards: [
				{ code: 'council_below_threshold', stops: 61, resolved: 55, mean_move_atr: -0.21 },
				{ code: 'members_split', stops: 22, resolved: 20, mean_move_atr: -0.1 },
				{ code: 'pre_close_cutoff', stops: 7, resolved: 6, mean_move_atr: 0.02 },
				{ code: 'sentinel_veto:max_open_positions', stops: 9, resolved: 9, mean_move_atr: 0.34 },
				{ code: 'event_blackout', stops: 5, resolved: 5, mean_move_atr: -0.48 }
			],
			proposal: {
				current: { ...live.weights }, proposed: { ...PROPOSED },
				changes: [
					{ agent: 'oracle', old: live.weights.oracle, new: PROPOSED.oracle, skill: 0.11, n: 84 },
					{ agent: 'chartist', old: live.weights.chartist, new: PROPOSED.chartist, skill: -0.03, n: 86 },
					{ agent: 'athena', old: live.weights.athena, new: PROPOSED.athena, skill: 0.02, n: 79 },
					{ agent: 'capitol', old: live.weights.capitol, new: PROPOSED.capitol, skill: 0.05, n: 37 }
				],
				holdout: { decisions: 31, brier_old: 0.248, brier_new: 0.241 },
				eligible: true,
				notes: ['Capitol has 37 resolved opinions: low sample, shrunk toward equal weights.', 'Every move is within the 0.05 per-review cap.']
			},
			live: structuredClone(live),
			candidates: { min_council_score: [0.25, 0.3, 0.35], max_open_positions: [1, 2, 3], no_new_trades_before_close_minutes: [5, 10, 15, 20], risk_percent: [0.25, 0.5, 0.75, 1], min_reward_risk: [1.25, 1.5, 2] },
			gates: {
				decisions_closed: closed, resolved_opinions: 84, resolved_since_last_review: sinceLast, last_review_ms: lastReviewMs, can_review: canReview,
				why_not: canReview ? null : `Next review after ${20 - sinceLast} more resolved opinions (every 20).`,
				min_decisions_tighten: 5, min_decisions_loosen: 100, min_guard_resolved_loosen: 20, review_every_resolved: 20
			}
		},
		reviews: structuredClone(reviews.slice(0, 5)),
		spend: { month: new Date().toISOString().slice(0, 7), spent_usd: round(spentUsd, 2), budget_usd: 2, warn: false, blocked: false },
		brain: { provider: 'claude', model: 'claude-sonnet-5-5', metered: true },
		brain_error: null,
		estimate_usd: 0.03
	};
}

export function architectureReviewRun() {
	if (sinceLast < 20) throw new ApiError('bad_request', `Next review after ${20 - sinceLast} more resolved opinions (every 20).`, 400);
	const r = makeReview();
	reviews.unshift(r);
	spentUsd += 0.031;
	sinceLast = 0;
	lastReviewMs = Date.now();
	return { review: structuredClone(r), spend: architectureLearning().spend };
}

function nextVersion(reason: string, proposalId: string | null, actor = 'you'): ConfigVersion {
	const v: ConfigVersion = {
		version: Math.max(...versions.map((x) => x.version)) + 1, created_at: new Date().toISOString(), weights: { ...live.weights }, guards: { ...live.guards },
		reason, proposal_id: proposalId, actor
	};
	versions.unshift(v);
	live = { ...live, version: v.version };
	return v;
}

export function architectureApply(body: ApplyBody, checkTotp: () => void) {
	const review = reviews.find((r) => r.id === body.review_id);
	const item: ReviewItem | undefined = review?.items.find((i) => i.id === body.item_id);
	if (!review || !item) throw new ApiError('not_found', 'no such suggestion', 404);
	if (item.applied_version !== null) throw new ApiError('bad_request', `already applied as v${item.applied_version}`, 400);
	if (item.locked) throw new ApiError('bad_request', item.locked, 400);
	if (item.direction === 'loosen') {
		if (!body.confirm_loosen) throw new ApiError('bad_request', `loosening ${item.param} needs a second confirm`, 400);
		checkTotp();
	}
	if (item.kind === 'weights') live = { ...live, weights: { ...(item.new as Weights) } };
	else live = { ...live, guards: { ...live.guards, [item.param]: item.new as number } };
	const v = nextVersion(`${item.param}: applied suggestion ${item.id}`, item.id);
	item.applied_version = v.version;
	return { version: structuredClone(v), live: structuredClone(live) };
}

// Looser means a lower floor or a higher cap.
const LOOSER: Partial<Record<keyof Guards, 'lower' | 'higher'>> = {
	min_council_score: 'lower', min_reward_risk: 'lower', min_non_neutral: 'lower', no_new_trades_before_close_minutes: 'lower',
	risk_percent: 'higher', max_open_positions: 'higher'
};

export function architectureVersions(): VersionList {
	return { live: structuredClone(live), defaults: structuredClone(DEFAULTS), versions: structuredClone(versions) };
}

export function architectureRollback(version: number, body: { confirm_loosen?: boolean; totp?: string }, checkTotp: () => void) {
	const target = versions.find((v) => v.version === version);
	if (!target) throw new ApiError('not_found', `no version ${version}`, 404);
	const looser = (Object.keys(LOOSER) as (keyof Guards)[]).filter((k) =>
		LOOSER[k] === 'lower' ? target.guards[k] < live.guards[k] : target.guards[k] > live.guards[k]
	);
	if (looser.length && !body.confirm_loosen)
		throw new ApiError('bad_request', `Rolling back to v${version} would loosen ${looser.map((k) => `${k} (${live.guards[k]} → ${target.guards[k]})`).join(', ')}`, 400);
	if (looser.length) checkTotp();
	live = { ...live, weights: { ...target.weights }, guards: { ...target.guards } };
	const v = nextVersion(`Rollback to v${version}`, null);
	return { version: structuredClone(v), live: structuredClone(live) };
}

// ---- Master account lock -----------------------------------------------------------------------

let lock = { expected: 53107904, broker: 'IC Markets' };

export function architectureLock(): LockView {
	const off = lock.expected === 0;
	return {
		expected: lock.expected,
		masked: maskAccount(lock.expected),
		broker: lock.broker,
		state: off ? 'off' : 'locked',
		message: off ? 'The lock is off: the Desk will trade on any MT5 account.' : 'MT5 reports the expected account.'
	};
}

export function architectureSetLock(body: LockBody, checkTotp: () => void): LockView {
	if (!Number.isInteger(body.expected) || body.expected < 0) throw new ApiError('bad_request', 'expected must be a whole account number (0 turns the lock off)', 400);
	if (body.expected !== body.confirm) throw new ApiError('bad_request', 'the two account numbers differ', 400);
	checkTotp();
	lock = { expected: body.expected, broker: body.broker?.trim() || lock.broker };
	return architectureLock();
}
