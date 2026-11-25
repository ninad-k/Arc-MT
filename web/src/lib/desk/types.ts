/**
 * Desk mode (spec-nimbus-desk) wire types: `GET /api/desk/state`, `GET /api/desk/pulse`, the
 * `desk` part of a chat reply, and `POST /api/desk/plans/{id}/pilot`.
 * Every figure may be `null`: the server sends `null` for "no data", and the HUD shows "—".
 */
import type { SpendStatus } from '#lib/api.ts';

export type AgentId =
	| 'friday'
	| 'atlas'
	| 'capitol'
	| 'scout'
	| 'athena'
	| 'chartist'
	| 'oracle'
	| 'sentinel'
	| 'pilot'
	| 'ledger';

export type SeatState = 'idle' | 'working' | 'done' | 'error' | 'offline';
export type AgentGroup = 'master' | 'intelligence' | 'analysis' | 'execution';

export interface Seat {
	id: AgentId;
	title: string;
	group: AgentGroup;
	state: SeatState;
	summary: string | null;
	at_ms: number | null;
	calls: number;
}

export interface Signal {
	at_ms: number;
	agent: AgentId;
	level: 'info' | 'ok' | 'warn' | 'error';
	text: string;
}

export interface Source {
	name: string;
	what: string;
	at: string | null;
}

export interface Finding {
	agent: AgentId;
	summary: string;
	data: Record<string, unknown>;
	sources: Source[];
	notes?: string[];
}

export type Phase = 'pre_market' | 'open' | 'after_hours' | 'closed';

export interface SessionStatus {
	phase: Phase;
	label: string;
	ny_time: string;
	ny_date: string;
	ny_zone: string;
	next_change: string | null;
	next_phase: Phase | null;
	seconds_to_change: number | null;
	seconds_to_close: number | null;
	trading_day: boolean;
	early_close: boolean;
}

export type MarkKind = 'fomc' | 'opex' | 'holiday' | 'early_close';

export interface Mark {
	date: string;
	kind: MarkKind;
	label: string;
	source: string;
}

export interface CalendarMonth {
	year: number;
	month: number;
	marks: Mark[];
}

export type Side = 'buy' | 'sell';

export interface Plan {
	symbol: string;
	broker_symbol: string;
	side: Side;
	entry: number;
	stop: number | null;
	target: number | null;
	volume: number;
	risk_percent: number;
	risk_amount: number | null;
	currency: string | null;
	sizing_error: string | null;
}

export type Verdict =
	| { decision: 'approved'; reasons: string[]; warnings: string[] }
	| { decision: 'vetoed'; reasons: string[] };

export interface PlanRecord {
	id: string;
	plan: Plan;
	verdict: Verdict;
	issued_ms: number;
	expires_ms: number;
	used: boolean;
}

export interface Bar {
	time: number;
	open: number;
	high: number;
	low: number;
	close: number;
	tick_volume?: number;
}

export type LevelKind = 'entry' | 'stop' | 'target' | 'supply' | 'demand' | 'liquidity' | 'marker';

export interface Level {
	kind: LevelKind;
	price: number;
	/** The top of a zone; `null` for a line. */
	to: number | null;
	label: string;
}

export interface Band {
	low: number;
	high: number;
	bars_ahead: number;
	label: string;
}

export interface Chart {
	symbol: string;
	timeframe: string;
	digits: number;
	bars: Bar[];
	levels: Level[];
	band: Band | null;
}

export interface JournalRow {
	id: number;
	ts: string;
	kind: string;
	text: string;
}

export interface DeskRules {
	risk_percent: number;
	max_open_positions: number;
	no_new_trades_before_close_minutes: number;
}

export interface DeskState {
	enabled: boolean;
	reason?: string;
	brain?: {
		provider: string;
		model: string | null;
		available: boolean;
		reason: string | null;
		metered: boolean;
	};
	session?: SessionStatus;
	calendar?: {
		today: string;
		months: CalendarMonth[];
		next_fomc: { date: string; source: string; days: number } | null;
	};
	seats?: Seat[];
	signals?: Signal[];
	plans?: PlanRecord[];
	journal?: JournalRow[];
	spend?: SpendStatus;
	finnhub?: boolean;
	rules?: DeskRules;
	kill_switch?: boolean;
}

export interface PulseQuote {
	symbol: string;
	price: number | null;
	change_pct: number | null;
}

export interface PulseResponse {
	quotes: PulseQuote[];
	at: string;
}

/** The `desk` part of a chat reply in Desk mode. */
export interface DeskTurn {
	findings: Finding[];
	chart: Chart | null;
	plans: PlanRecord[];
}

/** A `desk_activity` WebSocket event. */
export interface DeskActivityEvent {
	type: 'desk_activity';
	agent: AgentId;
	state: SeatState;
	summary: string | null;
	at_ms: number;
}
