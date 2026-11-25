<script lang="ts">
	import { desk } from '#lib/state/desk.svelte.ts';

	const rows = $derived(desk.state?.journal ?? []);
	const time = (ts: string) => {
		const d = new Date(ts);
		return Number.isNaN(d.getTime())
			? ts
			: d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
	};
</script>

{#if rows.length === 0}
	<p class="p-3 text-xs text-mute">Ledger keeps every desk step here: questions, findings, approvals, vetoes, previews.</p>
{:else}
	<ol class="divide-y divide-line/60" data-testid="journal">
		{#each rows as r (r.id)}
			<li class="flex gap-2 px-3 py-1.5 text-2xs">
				<span class="num shrink-0 text-faint">{time(r.ts)}</span>
				<span class="min-w-0 text-ink/90">{r.text}</span>
			</li>
		{/each}
	</ol>
{/if}
