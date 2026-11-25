import { api } from '#lib/api.ts';
import { RANGE_MS, type RangeId } from '#lib/architecture/format.ts';
import type {
	ArchitectureMap,
	DecisionDetail,
	DecisionFilter,
	DecisionSummary,
	Funnel,
	ReviewInput
} from '#lib/architecture/types.ts';
import { describeError, toApiError } from '#lib/errors.ts';
import { preview } from './preview.svelte';

/**
 * The Architecture page's data: the live map, the decision list and funnel, the open decision, and
 * the three actions (free scan, Ask Friday, save a review). Errors are text for the page to show.
 */
class ArchitectureStore {
	map = $state<ArchitectureMap | null>(null);
	mapError = $state<string | null>(null);

	decisions = $state<DecisionSummary[]>([]);
	listError = $state<string | null>(null);
	listLoading = $state(false);
	funnel = $state<Funnel | null>(null);
	funnelError = $state<string | null>(null);

	range = $state<RangeId>('24h');
	filter = $state<{ symbol: string; status: string; stage: string; stop: string }>({ symbol: '', status: '', stage: '', stop: '' });
	/** From the funnel: show only decisions that reached at least this step. */
	minStep = $state<number | null>(null);
	/** Every stop code seen, so the filter keeps its options once a code is chosen. */
	knownStops = $state<string[]>([]);

	/** The newest decision (Pipeline tab) and its full record. */
	latest = $state<DecisionDetail | null>(null);
	detail = $state<DecisionDetail | null>(null);
	detailError = $state<string | null>(null);
	detailLoading = $state(false);

	scanning = $state(false);
	scanError = $state<string | null>(null);
	scanNote = $state<string | null>(null);
	asking = $state(false);
	askError = $state<string | null>(null);
	askNote = $state<string | null>(null);
	reviewError = $state<string | null>(null);
	reviewSaving = $state(false);
	reviewSavedAt = $state<number | null>(null);

	private detailSeq = 0;

	get visible(): DecisionSummary[] {
		const k = this.minStep;
		return k === null ? this.decisions : this.decisions.filter((d) => d.step >= k);
	}

	private query(): DecisionFilter {
		const to = Date.now();
		return {
			from_ms: to - RANGE_MS[this.range],
			to_ms: to + 60_000,
			symbol: this.filter.symbol,
			status: this.filter.status as DecisionFilter['status'],
			stage: this.filter.stage as DecisionFilter['stage'],
			stop: this.filter.stop,
			limit: 200
		};
	}

	/** The range alone, for exports and the funnel. */
	rangeQuery(): DecisionFilter {
		const to = Date.now();
		return { from_ms: to - RANGE_MS[this.range], to_ms: to + 60_000 };
	}

	async loadMap(): Promise<void> {
		try {
			this.map = await api.architectureMap();
			this.mapError = null;
		} catch (e) {
			this.mapError = describeError(toApiError(e));
		}
	}

	async loadList(): Promise<void> {
		this.listLoading = true;
		try {
			const res = await api.architectureDecisions(this.query());
			this.decisions = res.decisions;
			this.listError = null;
			const known = new Set(this.knownStops);
			for (const d of res.decisions) if (d.stop_code) known.add(d.stop_code);
			this.knownStops = [...known].sort();
		} catch (e) {
			this.listError = describeError(toApiError(e));
		} finally {
			this.listLoading = false;
		}
	}

	async loadFunnel(): Promise<void> {
		try {
			this.funnel = await api.architectureFunnel(this.rangeQuery());
			this.funnelError = null;
			const known = new Set(this.knownStops);
			for (const s of this.funnel.steps) for (const c of Object.keys(s.dropped)) known.add(c);
			this.knownStops = [...known].sort();
		} catch (e) {
			this.funnelError = describeError(toApiError(e));
		}
	}

	/** Pipeline tab: the map plus the newest decision in full. */
	async loadPipeline(): Promise<void> {
		await Promise.all([this.loadMap(), this.loadLatest()]);
	}

	async loadLatest(): Promise<void> {
		try {
			const res = await api.architectureDecisions({ limit: 1 });
			const first = res.decisions[0];
			this.latest = first ? await api.architectureDecision(first.id) : null;
		} catch (e) {
			this.mapError = describeError(toApiError(e));
		}
	}

	async loadDecisions(): Promise<void> {
		await Promise.all([this.loadList(), this.loadFunnel()]);
	}

	async open(id: string | null): Promise<void> {
		const seq = ++this.detailSeq;
		if (!id) {
			this.detail = null;
			this.detailError = null;
			return;
		}
		if (this.detail?.id !== id) this.detail = null;
		this.detailLoading = true;
		this.askNote = null;
		this.askError = null;
		this.reviewError = null;
		try {
			const d = await api.architectureDecision(id);
			if (seq === this.detailSeq) {
				this.detail = d;
				this.detailError = null;
			}
		} catch (e) {
			if (seq === this.detailSeq) this.detailError = describeError(toApiError(e));
		} finally {
			if (seq === this.detailSeq) this.detailLoading = false;
		}
	}

	/** "Scan now": free, no LLM. Returns the first new decision id. */
	async scan(): Promise<string | null> {
		if (this.scanning) return null;
		this.scanning = true;
		this.scanError = null;
		this.scanNote = null;
		try {
			const res = await api.architectureScan();
			this.scanNote = `Scan done: ${res.ids.length} new ${res.ids.length === 1 ? 'decision' : 'decisions'}.`;
			await Promise.all([this.loadPipeline(), this.loadDecisions()]);
			return res.ids[0] ?? null;
		} catch (e) {
			this.scanError = describeError(toApiError(e));
			return null;
		} finally {
			this.scanning = false;
		}
	}

	/** "Ask Friday": a paid Desk turn. A returned preview opens the usual confirmation. */
	async ask(id: string): Promise<void> {
		if (this.asking) return;
		this.asking = true;
		this.askError = null;
		this.askNote = null;
		try {
			const res = await api.architectureAsk(id);
			this.detail = res.decision;
			this.askNote = res.reply;
			if (res.preview) preview.open(res.preview);
			void this.loadDecisions();
			void this.loadPipeline();
		} catch (e) {
			this.askError = describeError(toApiError(e));
		} finally {
			this.asking = false;
		}
	}

	async saveReview(id: string, review: ReviewInput): Promise<boolean> {
		this.reviewSaving = true;
		this.reviewError = null;
		try {
			const res = await api.architectureReview(id, review);
			if (this.detail?.id === id) this.detail = { ...this.detail, review: res.review, tags: [review.setup, ...review.mistakes].filter((t): t is string => !!t) };
			this.reviewSavedAt = Date.now();
			void this.loadList();
			return true;
		} catch (e) {
			this.reviewError = describeError(toApiError(e));
			return false;
		} finally {
			this.reviewSaving = false;
		}
	}
}

export const architecture = new ArchitectureStore();
