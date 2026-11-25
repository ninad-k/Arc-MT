<script lang="ts">
	import Panel from '#lib/components/Panel.svelte';
	import { desk } from '#lib/state/desk.svelte.ts';

	const seats = $derived((desk.state?.seats ?? []).filter((s) => s.id !== 'friday'));
	const max = $derived(Math.max(1, ...seats.map((s) => s.calls)));
	const spend = $derived(desk.state?.spend ?? null);
	const metered = $derived(desk.state?.brain?.metered === true);
</script>

<Panel title="Agent load">
	{#snippet meta()}
		<span>calls since start</span>
	{/snippet}
	<ul class="space-y-1 p-3" data-testid="agent-load">
		{#each seats as s (s.id)}
			<li class="flex items-center gap-2 text-2xs">
				<span class="w-16 shrink-0 tracking-wider text-mute uppercase">{s.id}</span>
				<span class="h-1.5 flex-1 bg-line/60">
					<span
						class="block h-full {s.state === 'error' ? 'bg-down' : s.state === 'offline' ? 'bg-faint' : 'bg-accent'}"
						style="width:{(s.calls / max) * 100}%"
					></span>
				</span>
				<span class="num w-6 shrink-0 text-right text-ink">{s.state === 'offline' ? '—' : s.calls}</span>
			</li>
		{/each}
	</ul>
	<div class="border-t border-line/70 px-3 py-2 text-2xs text-mute">
		{#if metered && spend}
			Claude this month:
			<span class="num {spend.blocked ? 'text-down' : spend.warn ? 'text-warn' : 'text-ink'}"
				>${spend.spent_usd.toFixed(2)} of ${spend.budget_usd.toFixed(2)}</span
			>
		{:else if desk.state?.brain}
			Local model: no Claude spending.
		{:else}
			—
		{/if}
		{#if desk.state && !desk.state.finnhub}
			<p class="mt-1 text-faint">No Finnhub key: insiders, fundamentals and filings show "—".</p>
		{/if}
	</div>
</Panel>
