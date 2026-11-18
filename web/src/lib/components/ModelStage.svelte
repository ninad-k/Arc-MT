<script lang="ts">
	/**
	 * Shows a Claude-built 3D model (spec-assistant-mode, ADR-0022). The model's code runs only
	 * inside `/sandbox/model.html`, loaded in an iframe with `sandbox="allow-scripts"`: an opaque
	 * origin with no network access, no access to this page, its cookies or its storage. The two
	 * sides talk only through `postMessage` (see `#lib/model3d/protocol.ts`). A watchdog removes
	 * the frame if the model stops responding.
	 */
	import { onDestroy } from 'svelte';
	import { api, type ModelRef } from '#lib/api.ts';
	import { describeError, toApiError } from '#lib/errors.ts';
	import { assistant } from '#lib/state/assistant.svelte.ts';
	import { parseFromSandbox, watchdog, type FileFormat, type ToSandbox } from '#lib/model3d/protocol.ts';

	let { model }: { model: ModelRef } = $props();

	type Phase = 'loading' | 'building' | 'ready' | 'error' | 'stuck';
	let phase = $state<Phase>('loading');
	let message = $state<string | null>(null);
	let triangles = $state(0);
	let frame = $state<HTMLIFrameElement>();
	/** Bumped to load a fresh frame (a new model, or "try again"). */
	let attempt = $state(0);

	/** Set once there is something to send to the frame (code, or an uploaded file). */
	let code = $state<string | null>(null);
	/** What goes to the frame: Claude's code, or an uploaded file or picture (bytes transferred). */
	let payload: ToSandbox | null = null;
	/** An uploaded file whose part names are not stored yet: save what the sandbox reports. */
	let needsInfo = false;
	let modelKind: 'code' | 'file' | 'relief' = 'code';
	let startedAt = 0;
	let lastHeard = 0;
	let ready = false;
	/** The code was sent to this frame (once per frame: a model cannot make the app resend it). */
	let sent = false;
	/** Page loads of this frame. The first is the sandbox page; any later one is the model
	 * navigating away, and the frame is closed. */
	let loads = 0;
	let request: AbortController | null = null;

	function send(msg: ToSandbox) {
		// The sandbox's origin is opaque, so it cannot be named; the code is not a secret.
		const transfer = msg.type === 'load' || msg.type === 'relief' ? [msg.bytes] : [];
		frame?.contentWindow?.postMessage(msg, '*', transfer);
	}

	async function load(m: ModelRef) {
		request?.abort();
		const ctrl = new AbortController();
		request = ctrl;
		phase = 'loading';
		message = null;
		code = null;
		payload = null;
		needsInfo = false;
		modelKind = 'code';
		ready = false;
		sent = false;
		loads = 0;
		try {
			const full = await api.model(m.id, ctrl.signal);
			if (ctrl.signal.aborted) return;
			modelKind = full.kind ?? 'code';
			if (modelKind === 'code') {
				payload = { type: 'run', code: full.code, title: full.title };
			} else {
				// An uploaded file or picture: fetch its bytes; the sandbox opens them.
				const bytes = await api.modelFile(m.id, ctrl.signal);
				if (ctrl.signal.aborted) return;
				payload =
					modelKind === 'relief'
						? { type: 'relief', bytes, overlay: full.overlay ?? null }
						: { type: 'load', format: (full.format ?? 'glb') as FileFormat, bytes, overlay: full.overlay ?? null };
				needsInfo = !full.info;
			}
			code = full.code;
			phase = 'building';
			startedAt = Date.now();
			lastHeard = startedAt;
			attempt++;
		} catch (e) {
			const err = toApiError(e);
			if (err.code === 'aborted') return;
			phase = 'error';
			message = describeError(err);
		}
	}

	$effect(() => {
		void load(model);
	});

	function onMessage(event: MessageEvent) {
		if (!frame || event.source !== frame.contentWindow) return;
		const msg = parseFromSandbox(event.data);
		if (!msg) return;
		lastHeard = Date.now();
		switch (msg.type) {
			case 'loaded':
				if (payload && !sent) {
					sent = true;
					startedAt = Date.now();
					send(payload);
					payload = null; // the bytes now belong to the frame
				}
				break;
			case 'ready':
				ready = true;
				triangles = msg.triangles;
				phase = 'ready';
				if (needsInfo && msg.info) {
					needsInfo = false;
					// Remembered so Claude can change it later; a failure here only costs that.
					void api.putModelInfo(model.id, msg.info).catch(() => {});
				}
				break;
			case 'error':
				phase = 'error';
				message =
					modelKind === 'code'
						? `The model's code failed: ${msg.message}`
						: `This file could not be shown: ${msg.message}`;
				break;
		}
	}

	function onFrameLoad() {
		loads++;
		if (loads > 1) {
			phase = 'stuck';
			message = 'The model tried to load another page, so it was closed.';
		}
	}

	const timer = setInterval(() => {
		// A hidden tab stops the frame's animation (and so its heartbeat): not a hang.
		if (document.hidden) return;
		if (phase !== 'building' && phase !== 'ready') return;
		if (watchdog(Date.now(), { startedAt, lastHeard, ready }) === 'stuck') {
			phase = 'stuck';
			message = 'This model stopped responding, so it was closed.';
		}
	}, 1000);

	function onVisibility() {
		send({ type: 'pause', paused: document.hidden });
		if (!document.hidden) {
			// Coming back: give the frame a fresh grace period before judging it.
			lastHeard = Date.now();
			if (!ready) startedAt = lastHeard;
		}
	}

	onDestroy(() => {
		clearInterval(timer);
		request?.abort();
	});
</script>

<svelte:window onmessage={onMessage} />
<svelte:document onvisibilitychange={onVisibility} />

<section class="relative flex h-full min-h-0 flex-col border border-line bg-[#05070d]" aria-label="3D model: {model.title}" data-testid="model-stage">
	<header class="flex items-center gap-2 border-b border-line px-3 py-1.5">
		<h2 class="min-w-0 flex-1 truncate font-mono text-xs font-semibold tracking-[0.16em] text-ink uppercase" title={model.title}>
			<span class="text-accent" aria-hidden="true">◆</span> {model.title}
		</h2>
		{#if phase === 'ready'}
			<span class="num text-2xs text-faint max-sm:hidden" title="Triangles in the model">{triangles.toLocaleString()} tris</span>
		{/if}
		{#if assistant.recent.length > 1}
			<label class="sr-only" for="model-pick">Other models</label>
			<select
				id="model-pick"
				class="num h-6 max-w-40 truncate border border-line bg-panel/70 px-1 text-2xs text-ink"
				value={model.id}
				onchange={(e) => {
					const m = assistant.recent.find((r) => r.id === e.currentTarget.value);
					if (m) assistant.show(m);
				}}
			>
				{#each assistant.recent as r (r.id)}
					<option value={r.id}>{r.title}</option>
				{/each}
			</select>
		{/if}
		<button
			type="button"
			class="border border-line px-2 py-0.5 text-2xs text-mute hover:border-accent hover:text-ink disabled:opacity-40"
			disabled={phase !== 'ready'}
			onclick={() => send({ type: 'reset-view' })}
		>
			Reset view
		</button>
		<button
			type="button"
			class="border border-line px-2 py-0.5 text-2xs text-mute hover:border-accent hover:text-ink"
			onclick={() => assistant.close()}
		>
			Close model
		</button>
	</header>

	<div class="relative min-h-0 flex-1">
		{#if (phase === 'building' || phase === 'ready') && code !== null}
			{#key attempt}
				<iframe
					bind:this={frame}
					src="/sandbox/model.html"
					sandbox="allow-scripts"
					referrerpolicy="no-referrer"
					title="3D model: {model.title}. Drag to rotate, scroll or pinch to zoom."
					class="absolute inset-0 h-full w-full border-0"
					data-testid="model-frame"
					onload={onFrameLoad}
				></iframe>
			{/key}
		{/if}
		{#if phase === 'loading' || phase === 'building'}
			<p class="pointer-events-none absolute inset-0 grid place-items-center text-xs text-mute" role="status">
				{phase === 'loading' ? 'Loading the model…' : 'Building the scene…'}
			</p>
		{:else if phase === 'error' || phase === 'stuck'}
			<div class="absolute inset-0 grid place-items-center p-4 text-center" role="alert">
				<div class="flex max-w-sm flex-col items-center gap-3">
					<p class="text-sm text-down">{message}</p>
					<button
						type="button"
						class="border border-line px-3 py-1 text-xs text-mute hover:border-accent hover:text-ink"
						onclick={() => void load(model)}
					>
						Try again
					</button>
				</div>
			</div>
		{/if}
	</div>
	<p class="border-t border-line px-3 py-1 text-2xs text-faint">
		Built by Claude. Drag to rotate, scroll or pinch to zoom. The model runs sealed off from the app, with no access to your data.
	</p>
</section>
