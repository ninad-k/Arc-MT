import { api, type ChatMode, type ChatResponse, type CitedNote, type ModelRef, type ToolCall } from '#lib/api.ts';
import { describeError, toApiError } from '#lib/errors.ts';
import { assistant } from './assistant.svelte';
import { mode } from './mode.svelte';
import { orb } from './orb.svelte';
import { persona } from './persona.svelte';
import { provider } from './provider.svelte';

export const MAX_MESSAGE = 2000;

export type ChatMessage =
	| { id: number; role: 'user'; text: string; at: number; voice: boolean }
	| {
			id: number;
			role: 'assistant';
			text: string;
			at: number;
			provider: string;
			toolCalls: ToolCall[];
			/** Display name of the persona that answered, kept so the label survives a later switch. */
			persona: string;
			/** Notes the reply drew on; shown as "from: <title>" chips that open the galaxy. */
			notes: CitedNote[];
			/** 3D models built in this turn (Assistant mode); shown as "3D: <title>" chips. */
			models: ModelRef[];
	  }
	| { id: number; role: 'error'; text: string; at: number; code: string }
	/** A line from the app itself: trade results, cancellations. */
	| { id: number; role: 'note'; text: string; at: number; tone: 'ok' | 'warn' | 'bad' };

/** One conversation: each mode keeps its own, so switching shows that mode's last chat. */
interface Thread {
	messages: ChatMessage[];
	sessionId: string | null;
	/** A reply is on its way. Per mode, so a slow 3D build never locks Trading. */
	pending: boolean;
}

class ChatStore {
	threads = $state<Record<ChatMode, Thread>>({
		trading: { messages: [], sessionId: null, pending: false },
		assistant: { messages: [], sessionId: null, pending: false }
	});

	private nextId = 1;

	/** The conversation of the current mode. */
	get messages(): ChatMessage[] {
		return this.threads[mode.value].messages;
	}

	get sessionId(): string | null {
		return this.threads[mode.value].sessionId;
	}

	/** A reply is on its way in the current mode. */
	get pending(): boolean {
		return this.threads[mode.value].pending;
	}

	/** Add what the user said or typed to the transcript without sending a chat request. */
	addUser(text: string, voice = false): void {
		this.messages.push({ id: this.nextId++, role: 'user', text, at: Date.now(), voice });
	}

	/** Add a reply that brought models to the stage (an upload, or Claude building from a picture). */
	addModels(text: string, models: ModelRef[]): void {
		this.messages.push({
			id: this.nextId++,
			role: 'assistant',
			text,
			at: Date.now(),
			provider: 'upload',
			persona: persona.name,
			toolCalls: [],
			notes: [],
			models
		});
	}

	/** Add a line written by the app itself (trade results, cancellations) to the transcript. */
	note(text: string, tone: 'ok' | 'warn' | 'bad' = 'ok'): void {
		this.messages.push({ id: this.nextId++, role: 'note', text, at: Date.now(), tone });
	}

	/**
	 * Send one turn to the chat endpoint. Returns the response, or null when nothing was sent or the
	 * call failed (the failure is already in the transcript). Previews and speech are handled by
	 * the caller (see `turn.svelte.ts`).
	 */
	async send(raw: string, opts: { voice?: boolean; alreadyShown?: boolean } = {}): Promise<ChatResponse | null> {
		const message = raw.trim();
		// The reply goes to the conversation it was asked in, even if the mode changes meanwhile.
		const turnMode = mode.value;
		const thread = this.threads[turnMode];
		if (thread.pending || message.length === 0 || message.length > MAX_MESSAGE) return null;

		if (!opts.alreadyShown) this.addUser(message, opts.voice === true);
		thread.pending = true;
		orb.set('thinking');

		try {
			const res = await api.chat({
				message,
				...(thread.sessionId ? { session_id: thread.sessionId } : {}),
				...(turnMode === 'assistant'
					? {
							mode: 'assistant' as const,
							...(assistant.stageModel ? { stage_model: { id: assistant.stageModel.id } } : {})
						}
					: provider.explicit
						? { provider: provider.selected ?? provider.explicit }
						: {}),
				...(opts.voice ? { voice: true } : {})
			});
			thread.sessionId = res.session_id;
			thread.messages.push({
				id: this.nextId++,
				role: 'assistant',
				text: res.reply,
				at: Date.now(),
				provider: res.provider,
				persona: persona.nameFor(res.persona),
				toolCalls: res.tool_calls ?? [],
				notes: res.notes ?? [],
				models: res.models ?? []
			});
			if (turnMode === 'assistant') assistant.onResponse(res);
			orb.set('idle');
			return res;
		} catch (e) {
			const err = toApiError(e);
			thread.messages.push({
				id: this.nextId++,
				role: 'error',
				text: err.isOffline
					? 'The Arc MT server is offline. Your message was not sent.'
					: describeError(err),
				at: Date.now(),
				code: err.code
			});
			orb.flashError();
			return null;
		} finally {
			thread.pending = false;
		}
	}

	/** Start a fresh session in the current mode (the server drops history on its own). */
	reset(): void {
		if (this.pending) return;
		this.threads[mode.value] = { messages: [], sessionId: null, pending: false };
	}
}

export const chat = new ChatStore();
