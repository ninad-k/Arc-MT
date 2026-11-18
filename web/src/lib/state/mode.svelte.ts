import type { ChatMode } from '#lib/api.ts';

const KEY = 'arc.mode';

function readStored(): ChatMode {
	try {
		return localStorage.getItem(KEY) === 'assistant' ? 'assistant' : 'trading';
	} catch {
		return 'trading';
	}
}

/**
 * Trading or Assistant (spec-assistant-mode). Trading is today's app: Jarvis or Friday on MT5.
 * Assistant is the Claude butler with notes, "remember that" and 3D models, and no MT5 tools.
 * Remembered per browser; a URL `?mode=assistant` or `?mode=trading` wins (used by tests).
 */
class ModeStore {
	value = $state<ChatMode>('trading');

	constructor() {
		if (typeof window === 'undefined') return;
		const fromUrl = new URLSearchParams(window.location.search).get('mode');
		this.value = fromUrl === 'assistant' || fromUrl === 'trading' ? fromUrl : readStored();
	}

	get assistant(): boolean {
		return this.value === 'assistant';
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
