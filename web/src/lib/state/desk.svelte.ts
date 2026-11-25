import { api, type ChatResponse } from '#lib/api.ts';
import type {
	AgentId,
	Chart,
	DeskActivityEvent,
	DeskState,
	Finding,
	PlanRecord,
	PulseQuote,
	SeatState
} from '#lib/desk/types.ts';
import { describeError, toApiError } from '#lib/errors.ts';
import { preview } from './preview.svelte';

const STATE_EVERY_MS = 10_000;
const PULSE_EVERY_MS = 20_000;
const MAX_QUEUE = 12;

export interface QueueItem {
	id: number;
	agent: AgentId;
	state: SeatState;
	at: number;
	summary: string | null;
}

/**
 * Desk mode (spec-nimbus-desk): the HUD's state (seats, signals, clock, calendar, plans, journal),
 * the market pulse, and what the last turn produced (findings, the chart Chartist drew).
 * Seats light up live from `desk_activity` events; the polls keep it right if the socket is down.
 */
class DeskStore {
	state = $state<DeskState | null>(null);
	/** When `state` arrived (its countdowns count from then). */
	loadedAt = $state(0);
	pulse = $state<PulseQuote[]>([]);
	pulseAt = $state<string | null>(null);
	chart = $state<Chart | null>(null);
	findings = $state<Finding[]>([]);
	queue = $state<QueueItem[]>([]);
	error = $state<string | null>(null);
	/** The plan id being sent to Pilot. */
	pilotBusy = $state<string | null>(null);
	/** Plans from replies, newest first, until the next state poll brings them too. */
	private extraPlans = $state<PlanRecord[]>([]);
	private nextId = 1;
	private loading = false;

	get plans(): PlanRecord[] {
		const fromState = this.state?.plans ?? [];
		const ids = new Set(fromState.map((p) => p.id));
		return [...this.extraPlans.filter((p) => !ids.has(p.id)), ...fromState];
	}

	async load(): Promise<void> {
		if (this.loading) return;
		this.loading = true;
		try {
			this.state = await api.deskState();
			this.loadedAt = Date.now();
			this.extraPlans = [];
			this.error = null;
		} catch (e) {
			this.error = describeError(toApiError(e));
		} finally {
			this.loading = false;
		}
	}

	async loadPulse(): Promise<void> {
		try {
			const res = await api.deskPulse();
			this.pulse = res.quotes;
			this.pulseAt = res.at;
		} catch {
			// The pulse keeps its last values; missing ones already show "—".
		}
	}

	/** Poll while the Desk page is on screen. Returns the stop function. */
	start(): () => void {
		void this.load();
		void this.loadPulse();
		const a = setInterval(() => void this.load(), STATE_EVERY_MS);
		const b = setInterval(() => void this.loadPulse(), PULSE_EVERY_MS);
		return () => {
			clearInterval(a);
			clearInterval(b);
		};
	}

	onResponse(res: ChatResponse): void {
		const turn = res.desk;
		if (!turn) return;
		this.findings = turn.findings ?? [];
		if (turn.chart) this.chart = turn.chart;
		if (turn.plans?.length) this.extraPlans = [...turn.plans, ...this.extraPlans];
		void this.load();
	}

	onActivity(e: DeskActivityEvent): void {
		const seats = this.state?.seats;
		const seat = seats?.find((s) => s.id === e.agent);
		if (seat) {
			if (e.state === 'working') seat.calls += 1;
			seat.state = e.state;
			seat.at_ms = e.at_ms;
			if (e.summary) seat.summary = e.summary;
		}
		if (e.agent === 'friday') return;
		if (e.state === 'working') {
			this.queue = [
				{ id: this.nextId++, agent: e.agent, state: 'working' as const, at: e.at_ms, summary: null },
				...this.queue
			].slice(0, MAX_QUEUE);
		} else {
			const item = this.queue.find((q) => q.agent === e.agent && q.state === 'working');
			if (item) {
				item.state = e.state;
				item.summary = e.summary;
			}
		}
	}

	onTradeExecuted(): void {
		void this.load();
	}

	/** The plan card's button: Pilot asks for a preview, which opens the usual confirmation. */
	async sendToPilot(planId: string): Promise<void> {
		if (this.pilotBusy) return;
		this.pilotBusy = planId;
		this.error = null;
		try {
			const p = await api.deskPilot(planId);
			preview.open(p);
		} catch (e) {
			this.error = describeError(toApiError(e));
		} finally {
			this.pilotBusy = null;
			void this.load();
		}
	}
}

export const desk = new DeskStore();
