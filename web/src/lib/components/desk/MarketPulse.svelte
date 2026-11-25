<script lang="ts">
	import Panel from '#lib/components/Panel.svelte';
	import { desk } from '#lib/state/desk.svelte.ts';
	import { direction, num, pct } from '#lib/desk/format.ts';

	/** Ring fill: a 3% move fills it. */
	const fill = (v: number | null) => (v === null ? 0 : Math.min(1, Math.abs(v) / 3));
	const C = 2 * Math.PI * 15;
</script>

<Panel title="Market pulse">
	{#snippet meta()}
		<span>{desk.pulseAt ? new Date(desk.pulseAt).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' }) : '—'}</span>
	{/snippet}
	{#if desk.pulse.length === 0}
		<p class="p-3 text-xs text-mute">No quotes yet. Add symbols to your watchlist or to [desk] indices.</p>
	{:else}
		<ul class="grid grid-cols-2 gap-2 p-2" data-testid="pulse">
			{#each desk.pulse as q (q.symbol)}
				{@const dir = direction(q.change_pct)}
				<li class="flex items-center gap-2 border border-line/70 bg-raise/40 px-2 py-1.5">
					<svg viewBox="0 0 36 36" class="size-9 shrink-0 -rotate-90" aria-hidden="true">
						<circle cx="18" cy="18" r="15" fill="none" stroke="var(--color-line)" stroke-width="3" />
						<circle
							cx="18"
							cy="18"
							r="15"
							fill="none"
							stroke={dir === 'down' ? 'var(--color-down)' : dir === 'up' ? 'var(--color-up)' : 'var(--color-faint)'}
							stroke-width="3"
							stroke-dasharray="{C * fill(q.change_pct)} {C}"
							stroke-linecap="round"
						/>
					</svg>
					<div class="min-w-0">
						<div class="truncate text-2xs font-semibold tracking-wider text-ink uppercase">{q.symbol}</div>
						<div class="num text-2xs text-mute">{num(q.price, q.price !== null && q.price < 10 ? 5 : 2)}</div>
						<div class="num text-2xs {dir === 'down' ? 'text-down' : dir === 'up' ? 'text-up' : 'text-faint'}">
							{pct(q.change_pct)}
						</div>
					</div>
				</li>
			{/each}
		</ul>
	{/if}
</Panel>
