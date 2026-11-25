<script lang="ts">
	import { provider } from '#lib/state/provider.svelte.ts';
	import { chat } from '#lib/state/chat.svelte.ts';
	import { mode } from '#lib/state/mode.svelte.ts';
	import { assistant } from '#lib/state/assistant.svelte.ts';
	import { desk } from '#lib/state/desk.svelte.ts';

	let { class: klass = '' }: { class?: string } = $props();
	const providers = $derived(provider.providers);
</script>

{#if mode.assistant}
	<!-- Assistant mode always answers with the configured Claude model: show it, no choice. -->
	{@const brain = assistant.status?.brain}
	<div class="flex min-w-0 items-center gap-2 {klass}" data-testid="brain-badge">
		<span class="hud-title max-[420px]:sr-only lg:max-xl:sr-only">Brain</span>
		<span
			class="num flex h-7 min-w-0 flex-1 items-center truncate border border-line bg-panel/70 px-2 text-2xs text-ink lg:w-44 lg:min-w-20 lg:flex-initial xl:w-56 xl:min-w-28"
			title="Assistant mode answers with this model (set in [assistant] of the server config)"
		>
			{brain ? `${brain.provider} · ${brain.model ?? '?'}` : 'Claude'}
		</span>
	</div>
{:else if mode.desk}
	<!-- Desk mode answers with the model in [desk] of the server config. -->
	{@const brain = desk.state?.brain}
	<div class="flex min-w-0 items-center gap-2 {klass}" data-testid="brain-badge">
		<span class="hud-title max-[420px]:sr-only lg:max-xl:sr-only">Brain</span>
		<span
			class="num flex h-7 min-w-0 flex-1 items-center truncate border border-line bg-panel/70 px-2 text-2xs text-ink lg:w-44 lg:min-w-20 lg:flex-initial xl:w-56 xl:min-w-28"
			title="Desk mode answers with this model (set in [desk] of the server config)"
		>
			{brain ? `${brain.provider} · ${brain.model ?? '?'}` : '—'}
		</span>
	</div>
{:else}
<div class="flex min-w-0 items-center gap-2 {klass}">
	<label for="llm-provider" class="hud-title max-[420px]:sr-only lg:max-xl:sr-only">LLM</label>
	<!-- Capped width, and on the one-row bar it gives way first (down to 5rem below xl, 7rem from xl) when the other
	     controls need the room: a long provider/model name must never push the page wider
	     than the window. The full list is also on the settings page. -->
	<select
		id="llm-provider"
		class="num h-7 min-w-0 flex-1 truncate border border-line bg-panel/70 px-2 text-2xs text-ink focus-visible:border-accent disabled:opacity-50 lg:w-44 lg:min-w-20 lg:flex-initial xl:w-56 xl:min-w-28"
		disabled={providers.length === 0 || chat.pending}
		value={provider.selected ?? ''}
		onchange={(e) => void provider.select(e.currentTarget.value)}
		title={chat.pending ? 'Wait for the current reply to finish' : 'Language model used for new messages'}
	>
		{#if providers.length === 0}
			<option value="">—</option>
		{/if}
		{#each providers as p (p.name)}
			<option value={p.name} disabled={!p.available}>
				{p.name} · {p.model}{p.available ? '' : ' — API key not set'}
			</option>
		{/each}
	</select>
</div>
{/if}
