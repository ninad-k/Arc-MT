<script lang="ts">
	import { uploads } from '#lib/state/uploads.svelte.ts';
	import { assistant } from '#lib/state/assistant.svelte.ts';

	let note = $state('');
	let dialog = $state<HTMLDivElement>();

	const pending = $derived(uploads.pendingImage);
	/** Claude is usable and the month's cap is not reached. */
	const claudeReady = $derived(
		assistant.status === null || (assistant.status.available && !assistant.status.spend?.blocked)
	);

	$effect(() => {
		if (pending) {
			note = '';
			queueMicrotask(() => dialog?.querySelector<HTMLElement>('textarea')?.focus());
		}
	});

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') uploads.cancelImage();
	}
</script>

{#if pending}
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<div class="fixed inset-0 z-40 grid place-items-center bg-void/70 p-4 backdrop-blur-sm" role="presentation" {onkeydown}>
		<div
			bind:this={dialog}
			role="dialog"
			aria-modal="true"
			aria-labelledby="image-sheet-title"
			class="flex w-full max-w-md flex-col gap-3 border border-line-hi bg-deck p-4 shadow-2xl"
			data-testid="image-sheet"
		>
			<h2 id="image-sheet-title" class="font-mono text-xs font-semibold tracking-[0.16em] text-ink uppercase">
				Your picture in 3D
			</h2>
			<img src={pending.url} alt="Your chosen file, preview" class="max-h-48 w-full border border-line object-contain" />
			<p class="truncate text-2xs text-faint" title={pending.file.name}>{pending.file.name}</p>

			<label for="image-note" class="hud-title">For Claude (optional)</label>
			<textarea
				id="image-note"
				bind:value={note}
				rows="2"
				maxlength="1000"
				placeholder="e.g. my 1 BHK floor plan, walls 3 m high"
				class="w-full resize-none border border-line-hi bg-void/70 px-2 py-1.5 text-sm text-ink placeholder:text-faint focus-visible:border-accent focus-visible:outline-none"
			></textarea>

			<div class="flex flex-col gap-2">
				<button
					type="button"
					class="border border-accent/60 bg-accent/15 px-3 py-2 text-left text-sm text-ink hover:bg-accent/25 disabled:cursor-not-allowed disabled:opacity-40"
					disabled={!claudeReady}
					onclick={() => void uploads.build(note)}
					data-testid="image-build"
				>
					Build a 3D model with Claude
					<span class="block text-2xs text-mute">
						Claude looks at the picture and builds what it shows. About $0.10–0.25; the picture is sent to Anthropic.
					</span>
				</button>
				{#if !claudeReady}
					<p class="text-2xs text-warn">Claude is not available right now (no API key, or the monthly cap is reached).</p>
				{/if}
				<button
					type="button"
					class="border border-line px-3 py-2 text-left text-sm text-ink hover:border-accent"
					onclick={() => void uploads.relief()}
					data-testid="image-relief"
				>
					Make a 3D relief
					<span class="block text-2xs text-mute">Free, done on this computer: the picture raised in depth, brighter parts higher.</span>
				</button>
				<button type="button" class="self-end px-2 py-1 text-xs text-mute hover:text-ink" onclick={() => uploads.cancelImage()}>
					Cancel
				</button>
			</div>
		</div>
	</div>
{/if}
