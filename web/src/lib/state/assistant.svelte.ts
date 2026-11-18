import {
	api,
	type AssistantStatus,
	type CapturedNote,
	type ChatResponse,
	type ModelRef,
	type ModelSummary
} from '#lib/api.ts';
import { describeError, toApiError } from '#lib/errors.ts';

/**
 * Assistant mode state (spec-assistant-mode): whether Claude is usable, this month's spending,
 * the 3D model on the stage, recent models, and the latest "remember that" note (the galaxy
 * listens for it to show the new star).
 */
class AssistantStore {
	status = $state<AssistantStatus | null>(null);
	statusError = $state<string | null>(null);
	/** The model shown on the stage, in place of the orb. */
	stageModel = $state<ModelRef | null>(null);
	recent = $state<ModelSummary[]>([]);
	lastCapture = $state<{ note: CapturedNote; at: number } | null>(null);

	async loadStatus(): Promise<void> {
		try {
			this.status = await api.assistantStatus();
			this.statusError = null;
		} catch (e) {
			this.statusError = describeError(toApiError(e));
		}
	}

	async loadRecent(): Promise<void> {
		try {
			this.recent = (await api.models()).models;
		} catch {
			// The list is a convenience; the stage still works without it.
		}
	}

	show(model: ModelRef): void {
		this.stageModel = { id: model.id, title: model.title };
	}

	close(): void {
		this.stageModel = null;
	}

	/** Take what an assistant reply carries: new models, captured notes, the spend. */
	onResponse(res: ChatResponse): void {
		if (res.spend && this.status) this.status = { ...this.status, spend: res.spend };
		const built = res.models ?? [];
		if (built.length > 0) {
			this.show(built[built.length - 1]);
			void this.loadRecent();
		}
		const captured = res.captured ?? [];
		if (captured.length > 0) this.lastCapture = { note: captured[captured.length - 1], at: Date.now() };
	}

	/** One line for the top of the chat, when something needs saying. */
	get notice(): { tone: 'warn' | 'bad'; text: string } | null {
		if (this.statusError) return { tone: 'bad', text: `Assistant status unavailable: ${this.statusError}` };
		const s = this.status;
		if (!s) return null;
		if (!s.enabled) return { tone: 'bad', text: 'Assistant mode is turned off in the server config ([assistant] enabled = false).' };
		if (!s.available)
			return {
				tone: 'bad',
				text: `Claude is not set up (${s.reason ?? 'unavailable'}). Set ANTHROPIC_API_KEY for the server and restart it.`
			};
		const spend = s.spend;
		if (spend?.blocked)
			return {
				tone: 'bad',
				text: `This month's Claude budget is used up ($${spend.spent_usd.toFixed(2)} of $${spend.budget_usd.toFixed(2)}).`
			};
		if (spend?.warn)
			return {
				tone: 'warn',
				text: `Claude spending this month: $${spend.spent_usd.toFixed(2)} of $${spend.budget_usd.toFixed(2)}.`
			};
		return null;
	}
}

export const assistant = new AssistantStore();
