<script lang="ts">
	import { stepStrip, stripLabel } from '#lib/architecture/format.ts';
	import type { DecisionStatus } from '#lib/architecture/types.ts';

	let { step, status }: { step: number; status: DecisionStatus } = $props();
	const dots = $derived(stepStrip(step, status));
	const cls = { p: 'bg-up', s: 'bg-down', w: 'bg-warn', n: 'bg-line' } as const;
</script>

<span class="inline-flex items-center gap-[3px]" role="img" aria-label={stripLabel(step, status)}>
	{#each dots as d, i (i)}<i class="block h-1.5 w-3.5 rounded-full {cls[d]}"></i>{/each}
</span>
