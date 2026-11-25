/**
 * Architecture page (spec-architecture-page) wire types: `GET /api/architecture/map`, the decision
 * list and record, the funnel, the manual review and "Ask Friday".
 */
import type { PreviewDto } from '#lib/api.ts';
import type { PlanRecord, Seat, SessionStatus, Source } from '#lib/desk/types.ts';

export type Stance = 'bull' | 'bear' | 'neutral';
export type MemberId = 'atlas' | 'capitol' | 'scout' | 'athena' | 'chartist' | 'oracle';
export type ExecId = 'sentinel' | 'pilot' | 'gate' | 'confirm' | 'broker';
export type CouncilSeat = 'bull_case' | 'bear_case' | 'risk' | 'chair';
export type Venue = 'mt5' | 'alpaca';
export type Stage = 'signal' | 'members' | 'council' | 'master' | 'execution';
export type DecisionStatus = 'open' | 'waiting' | 'traded' | 'stopped';
/** 0 signal, 1 members ran, 2 council passed, 3 plan, 4 preview, 5 traded. */
export type Step = 0 | 1 | 2 | 3 | 4 | 5;

export interface LatestOpinion {
	agent: MemberId;
	stance: Stance;
	/** 0..100 */
	confidence: number;
	weight: number;
	contribution: number;
	evidence: string[];
	symbol: string;
	at: string;
	decision_id: string;
	opinions: number;
}

export interface MapMember {
	agent: MemberId;
	title: string;
	group: string;
	weight: number;
	latest: LatestOpinion | null;
	seat: Seat | null;
}

export interface Guards {
	risk_percent: number;
	max_open_positions: number;
	no_new_trades_before_close_minutes: number;
	min_reward_risk: number;
	min_council_score: number;
	min_non_neutral: number;
}

export interface CouncilWeights {
	atlas: number;
	capitol: number;
	scout: number;
	athena: number;
	chartist: number;
	oracle: number;
}

export interface LiveConfig {
	version: number | null;
	weights: CouncilWeights;
	guards: Guards;
}

export interface MasterNode {
	agent: 'friday';
	provider: string;
	model: string | null;
	seat: Seat | null;
	spend: { month: string; spent_usd: number; budget_usd: number; warn: boolean; blocked: boolean } | null;
}

export interface ExecNode {
	id: ExecId;
	seat: Seat | null;
}

export interface ArchitectureMap {
	members: MapMember[];
	council: CouncilSeat[];
	master: MasterNode;
	execution: ExecNode[];
	live: LiveConfig;
	defaults: LiveConfig;
	session: SessionStatus;
	scan_every_minutes: number;
	kill_switch: boolean;
}

export interface Trigger {
	kind: 'scan' | 'question' | 'ask';
	detail: string | null;
}

export interface DecisionSummary {
	id: string;
	created_at: string;
	created_ms: number;
	symbol: string;
	venue: Venue;
	trigger: Trigger;
	stage: Stage;
	status: DecisionStatus;
	stop_code: string | null;
	stop_label: string | null;
	stop_detail: string | null;
	score: number | null;
	threshold: number | null;
	council_passed: boolean;
	side: 'bull' | 'bear' | null;
	has_plan: boolean;
	has_preview: boolean;
	result_r: number | null;
	step: Step;
	tags: string[];
}

export interface Contribution {
	agent: MemberId;
	weight: number;
	stance: Stance;
	confidence: number;
	value: number;
}

export interface CasePoint {
	agent: MemberId;
	value: number;
	evidence: string;
}

export interface CouncilCase {
	total: number;
	points: CasePoint[];
}

export interface CouncilRecord {
	score: number;
	threshold: number;
	non_neutral: number;
	contributions: Contribution[];
	bull: CouncilCase;
	bear: CouncilCase;
	risk: { rule: string; reason: string }[];
	passed: boolean;
	side: 'bull' | 'bear' | null;
	stop: [string, string] | null;
}

export type Message =
	| { role: 'user'; text: string }
	| { role: 'assistant'; text: string | null; tool_calls: { id: string; name: string; arguments: unknown }[] }
	| { role: 'tool_result'; call_id: string; name: string; content: string; is_error: boolean };

export interface MasterRecord {
	provider: string;
	model: string;
	system_hash: string;
	system: string;
	messages: Message[];
	reply: string;
	usage: { input_tokens: number; output_tokens: number; cost_usd: number };
}

export interface MasterError {
	error: string;
}

export interface DecisionPlan {
	symbol: string;
	broker_symbol: string;
	side: 'buy' | 'sell';
	entry: number;
	stop: number | null;
	target: number | null;
	volume: number;
	risk_percent: number;
	risk_amount: number | null;
	currency: string | null;
	sizing_error: string | null;
}

export type DecisionVerdict =
	| { decision: 'approved'; reasons: string[]; warnings: string[] }
	| { decision: 'vetoed'; reasons: string[]; rules: string[] };

export interface OpinionRecord {
	agent: MemberId;
	stance: Stance;
	/** 0..100 */
	confidence: number;
	weight: number;
	contribution: number;
	horizon: string;
	evidence: string[];
	sources: Source[];
	data: Record<string, unknown> | null;
	mapper: string;
	outcome: string | null;
	move_atr: number | null;
}

export interface ReviewInput {
	setup: string | null;
	mistakes: string[];
	regime: string | null;
	/** 1..5 */
	rating: number | null;
	take_again: boolean | null;
	notes: string | null;
}

export interface Review extends ReviewInput {
	updated_at?: string | null;
}

export interface DecisionDetail extends DecisionSummary {
	session_id: string | null;
	broker_symbol: string | null;
	price: number | null;
	atr: number | null;
	council: CouncilRecord | null;
	weights_version: number | null;
	master: MasterRecord | MasterError | null;
	plan_id: string | null;
	plan: DecisionPlan | null;
	verdict: DecisionVerdict | null;
	preview_id: string | null;
	tickets: number[];
	pnl: number | null;
	closed_at: string | null;
	opinions: OpinionRecord[];
	review: Review | null;
}

export interface DecisionFilter {
	from_ms?: number;
	to_ms?: number;
	symbol?: string;
	status?: DecisionStatus | '';
	stage?: Stage | '';
	stop?: string;
	limit?: number;
}

export interface FunnelStep {
	step: Step;
	label: string;
	count: number;
	dropped: Record<string, number>;
}

export interface Funnel {
	from_ms: number;
	to_ms: number;
	steps: FunnelStep[];
}

export interface ScanResponse {
	ids: string[];
	decisions: DecisionSummary[];
}

export interface AskResponse {
	session_id: string;
	reply: string;
	preview: PreviewDto | null;
	plans: PlanRecord[];
	spend: unknown;
	decision: DecisionDetail;
}


// ---- Learning loop and master account lock ----------------------------------------------------

export type Weights = CouncilWeights;

export interface MemberStats {
	agent: MemberId;
	n: number | null;
	n_taken: number | null;
	n_counterfactual: number | null;
	neutral: number | null;
	flat: number | null;
	hit_rate: number | null;
	hit_rate_taken: number | null;
	hit_rate_counterfactual: number | null;
	brier: number | null;
	/** -1..1; 0 is a coin. */
	skill: number | null;
	skill_low: number | null;
	skill_high: number | null;
}

export interface GuardStats {
	code: string;
	stops: number | null;
	resolved: number | null;
	/** The move in the council's direction after the stop, in ATR: positive = the stop missed a move. */
	mean_move_atr: number | null;
}

export interface WeightChange {
	agent: MemberId;
	old: number;
	new: number;
	skill: number | null;
	n: number | null;
}

export interface WeightProposal {
	current: Weights;
	proposed: Weights;
	changes: WeightChange[];
	holdout: { decisions: number; brier_old: number | null; brier_new: number | null } | null;
	eligible: boolean;
	notes: string[];
}

export interface LearningGates {
	decisions_closed: number;
	resolved_opinions: number;
	resolved_since_last_review: number;
	last_review_ms: number | null;
	can_review: boolean;
	why_not: string | null;
	min_decisions_tighten: number;
	min_decisions_loosen: number;
	min_guard_resolved_loosen: number;
	review_every_resolved: number;
}

export interface LearningSnapshot {
	members: MemberStats[];
	guards: GuardStats[];
	proposal: WeightProposal;
	live: LiveConfig;
	candidates: Record<string, number[]>;
	gates: LearningGates;
}

export type ItemDirection = 'reweight' | 'tighten' | 'loosen';

export interface ReviewItem {
	id: string;
	kind: 'weights' | 'guard';
	param: string;
	/** Weight maps for weights items, numbers for guard items. */
	old: Weights | number;
	new: Weights | number;
	direction: ItemDirection;
	rationale: string;
	evidence_refs: string[];
	confidence: number;
	locked: string | null;
	applied_version: number | null;
}

export interface LearningReview {
	id: string;
	created_at: string;
	model: string;
	summary: string;
	items: ReviewItem[];
	rejected: { item: unknown; reason: string }[];
	cost_usd: number;
	live_version: number | null;
}

export interface SpendLike {
	month: string;
	spent_usd: number;
	budget_usd: number;
	warn: boolean;
	blocked: boolean;
}

export interface LearningResponse {
	snapshot: LearningSnapshot;
	reviews: LearningReview[];
	spend: SpendLike | null;
	brain: { provider: string; model: string | null; metered: boolean } | null;
	brain_error: string | null;
	estimate_usd: number;
}

export interface ConfigVersion {
	version: number;
	created_at: string;
	weights: Weights;
	guards: Guards;
	reason: string | null;
	proposal_id: string | null;
	actor: string | null;
}

export interface VersionList {
	live: LiveConfig;
	defaults: LiveConfig;
	versions: ConfigVersion[];
}

export interface ApplyBody {
	review_id: string;
	item_id: string;
	confirm_loosen?: boolean;
	totp?: string;
}

export type LockState = 'locked' | 'mismatch' | 'unverified' | 'off';

export interface LockView {
	expected: number;
	masked: string;
	broker: string;
	state: LockState;
	message: string;
}

export interface LockBody {
	expected: number;
	confirm: number;
	broker?: string;
	totp?: string;
}
