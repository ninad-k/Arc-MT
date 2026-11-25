<script lang="ts">
	import { desk } from '#lib/state/desk.svelte.ts';

	const time = (iso: string | null) =>
		iso ? new Date(iso).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' }) : '—';
</script>

{#if desk.findings.length === 0}
	<p class="p-3 text-xs text-mute">What each seat found for your last question appears here, with its sources.</p>
{:else}
	<ul class="space-y-2 p-2" data-testid="findings">
		{#each desk.findings as f, i (i)}
			<li class="border border-line/70 px-3 py-2 text-xs">
				<p class="text-2xs font-semibold tracking-wider text-accent uppercase">{f.agent}</p>
				<p class="mt-0.5 text-ink">{f.summary}</p>
				{#if f.notes?.length}
					<ul class="mt-1 text-2xs text-warn">
						{#each f.notes as n, j (j)}<li>{n}</li>{/each}
					</ul>
				{/if}
				<p class="mt-1 text-2xs text-faint">
					{#each f.sources as s, j (j)}<span class="mr-2">{s.name}: {s.what} ({time(s.at)})</span>{/each}
				</p>
			</li>
		{/each}
	</ul>
{/if}
