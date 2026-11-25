<script lang="ts">
	import { contributionBar, signed, thresholdLines } from '#lib/architecture/format.ts';
	import type { Contribution } from '#lib/architecture/types.ts';

	let {
		contributions,
		score,
		threshold,
		scale = 0.5
	}: { contributions: Contribution[]; score: number; threshold: number; scale?: number } = $props();

	const lines = $derived(thresholdLines(threshold, scale));
	const total = $derived(contributionBar(score, scale));
	const name = (a: string) => a.charAt(0).toUpperCase() + a.slice(1);
	const cls = (v: number) => (v > 0 ? 'bg-up' : v < 0 ? 'bg-down' : 'bg-line-hi');
	const txt = (v: number) => (v > 0 ? 'text-up' : v < 0 ? 'text-down' : 'text-mute');
</script>

<div class="grid grid-cols-[6.5rem_minmax(0,1fr)_3.5rem] items-center gap-x-2 gap-y-1.5 text-xs" data-testid="contribution-bars">
	{#each contributions as c (c.agent)}
		{@const b = contributionBar(c.value, scale)}
		<span class="truncate text-mute">
			{name(c.agent)} <span class="num text-faint">{c.stance === 'neutral' ? 'neutral' : `${c.weight.toFixed(2)}×${Math.round(c.confidence)}`}</span>
		</span>
		<span class="relative block h-3.5">
			<span class="absolute inset-y-0 left-1/2 w-px bg-line-hi"></span>
			<span class="absolute top-0.5 h-2.5 {cls(c.value)}" style="left:{b.left}%;width:{b.width}%"></span>
		</span>
		<span class="num text-right {txt(c.value)}">{signed(c.value, 3)}</span>
	{/each}
	<b class="text-ink">Score</b>
	<span class="relative block h-3.5">
		<span class="absolute inset-y-0 left-1/2 w-px bg-line-hi"></span>
		<span class="absolute top-0.5 h-2.5 bg-accent" style="left:{total.left}%;width:{total.width}%"></span>
		<span class="absolute -top-0.5 -bottom-0.5 border-l border-dashed border-warn" style="left:{lines[0]}%" title="-{threshold.toFixed(2)}"></span>
		<span class="absolute -top-0.5 -bottom-0.5 border-l border-dashed border-warn" style="left:{lines[1]}%" title="+{threshold.toFixed(2)}"></span>
	</span>
	<b class="num text-right text-accent" data-testid="council-score">{signed(score, 3)}</b>
</div>
<p class="num mt-1 text-right text-2xs text-warn">┆ threshold ±{threshold.toFixed(2)}</p>
