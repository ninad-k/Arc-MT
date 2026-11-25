<script lang="ts">
	import type { Snippet } from 'svelte';
	import { bannerHead, signed, stopLabel } from '#lib/architecture/format.ts';
	import type { DecisionSummary } from '#lib/architecture/types.ts';

	let { decision, children }: { decision: DecisionSummary; children?: Snippet } = $props();

	const tone = $derived(decision.status === 'traded' ? 'up' : decision.status === 'stopped' ? 'down' : 'warn');
	const box = $derived(
		tone === 'up'
			? 'border-up/50 bg-up/5'
			: tone === 'down'
				? 'border-down/50 bg-down/5'
				: 'border-warn/50 bg-warn/5'
	);
	const head = $derived(tone === 'up' ? 'text-up' : tone === 'down' ? 'text-down' : 'text-warn');
	const reason = $derived(
		decision.status === 'traded'
			? null
			: `${stopLabel(decision.stop_code, decision.stop_label)}${
					decision.stop_code === 'council_below_threshold' && decision.score !== null && decision.threshold !== null
						? ` (${signed(decision.score)} vs ${decision.threshold.toFixed(2)})`
						: ''
				}`
	);
</script>

<div class="flex flex-wrap items-center gap-3 border px-3 py-2 {box}" role="status" data-testid="stop-banner">
	<div class="min-w-0 flex-1 text-xs">
		<p class="font-semibold {head}">
			{bannerHead(decision)}
			{#if decision.stop_code}<span class="num ml-2 font-normal text-faint" data-testid="stop-code">{decision.stop_code}</span>{/if}
		</p>
		{#if reason}<p class="mt-0.5 text-ink" data-testid="stop-reason">{reason}</p>{/if}
		{#if decision.stop_detail && decision.stop_detail !== reason}<p class="mt-0.5 text-mute">{decision.stop_detail}</p>{/if}
	</div>
	{@render children?.()}
</div>
