import { api } from '#lib/api.ts';
import type {
	ApplyBody,
	LearningResponse,
	LockBody,
	LockView,
	ReviewItem,
	VersionList
} from '#lib/architecture/types.ts';
import { describeError, toApiError } from '#lib/errors.ts';

/** Server message for an error; a wrong authenticator code is a 401 that is an answer. */
export function errText(e: unknown): string {
	const err = toApiError(e);
	if (err.code === 'unauthorized') return err.message && err.message !== 'Sign-in required' ? err.message : 'Code not accepted';
	return describeError(err);
}

export type ActionResult = { ok: true } | { ok: false; message: string; status: number };

/**
 * The Learning tab and the master account lock: the statistics and suggestions, applying and
 * rolling back (Claude never writes; every change is a click here), and the lock view.
 */
class LearningStore {
	data = $state<LearningResponse | null>(null);
	error = $state<string | null>(null);
	loading = $state(false);
	versions = $state<VersionList | null>(null);
	versionsError = $state<string | null>(null);
	/** Suggestions the user hid (client-side only). */
	dismissed = $state<string[]>([]);

	running = $state(false);
	runError = $state<string | null>(null);

	/** Whether the server wants an authenticator code; null until known. */
	authEnabled = $state<boolean | null>(null);

	lock = $state<LockView | null>(null);
	lockError = $state<string | null>(null);

	async loadAuth(): Promise<void> {
		if (this.authEnabled !== null) return;
		try {
			const s = await api.status();
			this.authEnabled = s.auth ? s.auth.enabled : null;
		} catch {
			// The field stays optional.
		}
	}

	async load(): Promise<void> {
		this.loading = true;
		try {
			const [d, v] = await Promise.allSettled([api.architectureLearning(), api.architectureVersions()]);
			if (d.status === 'fulfilled') {
				this.data = d.value;
				this.error = null;
			} else this.error = errText(d.reason);
			if (v.status === 'fulfilled') {
				this.versions = v.value;
				this.versionsError = null;
			} else this.versionsError = errText(v.reason);
		} finally {
			this.loading = false;
		}
	}

	/** "Run review": a paid Claude turn. */
	async runReview(): Promise<void> {
		if (this.running) return;
		this.running = true;
		this.runError = null;
		try {
			await api.architectureReviewRun();
			this.dismissed = [];
			await this.load();
		} catch (e) {
			this.runError = errText(e);
		} finally {
			this.running = false;
		}
	}

	dismiss(itemId: string): void {
		if (!this.dismissed.includes(itemId)) this.dismissed = [...this.dismissed, itemId];
	}

	async apply(reviewId: string, item: ReviewItem, opts: { confirm_loosen?: boolean; totp?: string } = {}): Promise<ActionResult> {
		const body: ApplyBody = { review_id: reviewId, item_id: item.id };
		if (opts.confirm_loosen) body.confirm_loosen = true;
		if (opts.totp) body.totp = opts.totp;
		try {
			await api.architectureApply(body);
			await this.load();
			return { ok: true };
		} catch (e) {
			return { ok: false, message: errText(e), status: toApiError(e).status };
		}
	}

	async rollback(version: number, opts: { confirm_loosen?: boolean; totp?: string } = {}): Promise<ActionResult> {
		const body: { confirm_loosen?: boolean; totp?: string } = {};
		if (opts.confirm_loosen) body.confirm_loosen = true;
		if (opts.totp) body.totp = opts.totp;
		try {
			await api.architectureRollback(version, body);
			await this.load();
			return { ok: true };
		} catch (e) {
			return { ok: false, message: errText(e), status: toApiError(e).status };
		}
	}

	async loadLock(): Promise<void> {
		try {
			this.lock = await api.architectureLock();
			this.lockError = null;
		} catch (e) {
			this.lockError = errText(e);
		}
	}

	async saveLock(body: LockBody): Promise<ActionResult> {
		try {
			this.lock = await api.architectureSetLock(body);
			this.lockError = null;
			return { ok: true };
		} catch (e) {
			return { ok: false, message: errText(e), status: toApiError(e).status };
		}
	}
}

export const learning = new LearningStore();
