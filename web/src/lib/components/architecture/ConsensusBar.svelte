<script lang="ts">
	import { consensus } from '#lib/architecture/format.ts';
	import type { Stance } from '#lib/architecture/types.ts';

	let { items }: { items: { stance: Stance; confidence: number; weight: number }[] } = $props();
	const c = $derived(consensus(items));
	const label = $derived(`Bull ${c.bull}%, neutral ${c.neutral}%, bear ${c.bear}%`);
</script>

<div data-testid="consensus-bar">
	<div class="flex h-3 overflow-hidden border border-line" role="img" aria-label={label}>
		<span class="block bg-up" style="width:{c.bull}%"></span>
		<span class="block bg-mute/60" style="width:{c.neutral}%"></span>
		<span class="block bg-down" style="width:{c.bear}%"></span>
	</div>
	<div class="num mt-1 flex justify-between text-2xs">
		<span class="text-up">Bull {c.bull}%</span>
		<span class="text-mute">Neutral {c.neutral}%</span>
		<span class="text-down">Bear {c.bear}%</span>
	</div>
</div>
