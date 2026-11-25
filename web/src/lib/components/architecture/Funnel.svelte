<script lang="ts">
	import { dropChips, funnelWidth } from '#lib/architecture/format.ts';
	import type { Funnel } from '#lib/architecture/types.ts';

	let {
		funnel,
		active,
		onstep,
		onstop
	}: {
		funnel: Funnel | null;
		active: number | null;
		onstep: (step: number | null) => void;
		onstop: (code: string) => void;
	} = $props();

	const first = $derived(funnel?.steps[0]?.count ?? 0);
	const last = $derived(funnel?.steps.at(-1)?.count ?? 0);
</script>

<div data-testid="funnel">
	<p class="mb-2 text-right text-2xs text-faint">
		{#if funnel}{first} {first === 1 ? 'signal' : 'signals'} → {last} {last === 1 ? 'trade' : 'trades'}{/if}
	</p>
	<ol class="grid grid-cols-2 gap-2 sm:grid-cols-3 xl:grid-cols-6">
		{#if !funnel}
			{#each Array(6) as _, i (i)}<li class="skeleton h-24"></li>{/each}
		{:else}
			{#each funnel.steps as s (s.step)}
				{@const chips = dropChips(s.dropped)}
				<li
					class="border bg-deck p-2 {active === s.step ? 'border-accent' : 'border-line'}"
					data-testid="funnel-step-{s.step}"
				>
					<button
						type="button"
						class="block w-full text-left hover:text-ink"
						aria-pressed={active === s.step}
						aria-label="{s.label}: {s.count}. Filter the table to decisions that reached this stage"
						onclick={() => onstep(active === s.step ? null : s.step)}
					>
						<span class="block text-2xs text-faint">{s.label}</span>
						<span class="num block text-xl font-semibold text-ink">{s.count}</span>
						<span class="my-1 block h-1.5 bg-accent" style="width:{funnelWidth(s.count, first)}%"></span>
					</button>
					<div class="mt-1 flex flex-wrap gap-1">
						{#each chips as c (c.code)}
							<button
								type="button"
								class="num border border-down/40 px-1 text-[10.5px] text-down/90 hover:border-down hover:text-down"
								title="{c.label}. Filter the table to this stop code"
								onclick={() => onstop(c.code)}
							>
								{c.code} {c.count}
							</button>
						{/each}
					</div>
				</li>
			{/each}
		{/if}
	</ol>
</div>
