import type { ChatMode } from '#lib/api.ts';

const KEY = 'arc.mode';

const MODES: readonly ChatMode[] = ['trading', 'assistant', 'desk'];

export function parseMode(v: string | null | undefined): ChatMode | null {
	return MODES.find((m) => m === v) ?? null;
}

function readStored(): ChatMode {
	try {
		return parseMode(localStorage.getItem(KEY)) ?? 'trading';
	} catch {
		return 'trading';
	}
}

/**
 * Trading, Assistant (spec-assistant-mode) or Desk (spec-nimbus-desk). Trading is today's app:
 * Jarvis or Friday on MT5. Assistant is the Claude butler with notes, "remember that" and 3D
 * models, and no MT5 tools. Desk is Nimbus Desk: Friday and eight specialists, trades only through
 * Sentinel, Pilot and the usual confirmation.
 * Remembered per browser; a URL `?mode=desk` (or `assistant`, `trading`) wins (used by tests).
 */
class ModeStore {
	value = $state<ChatMode>('trading');

	constructor() {
		if (typeof window === 'undefined') return;
		const fromUrl = new URLSearchParams(window.location.search).get('mode');
		this.value = parseMode(fromUrl) ?? readStored();
	}

	get assistant(): boolean {
		return this.value === 'assistant';
	}

	get desk(): boolean {
		return this.value === 'desk';
	}

	set(next: ChatMode): void {
		this.value = next;
		try {
			localStorage.setItem(KEY, next);
		} catch {
			// Private windows and blocked storage: the switch still works for this visit.
		}
	}
}

export const mode = new ModeStore();
