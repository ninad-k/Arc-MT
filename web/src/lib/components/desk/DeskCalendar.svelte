<script lang="ts">
	import Panel from '#lib/components/Panel.svelte';
	import { desk } from '#lib/state/desk.svelte.ts';
	import { MONTHS, monthGrid } from '#lib/desk/format.ts';
	import type { MarkKind } from '#lib/desk/types.ts';

	let index = $state(0);
	const cal = $derived(desk.state?.calendar ?? null);
	const month = $derived(cal?.months[index] ?? null);
	const weeks = $derived(month && cal ? monthGrid(month.year, month.month, month.marks, cal.today) : []);
	const dot: Record<MarkKind, string> = {
		fomc: 'bg-accent',
		opex: 'bg-violet',
		holiday: 'bg-down',
		early_close: 'bg-warn'
	};
	const legend: [MarkKind, string][] = [
		['fomc', 'FOMC'],
		['opex', 'Opex'],
		['holiday', 'Holiday'],
		['early_close', 'Early close']
	];
</script>

<Panel title="Calendar">
	{#snippet meta()}
		{#if cal && cal.months.length > 1}
			<button
				type="button"
				class="px-1 text-mute hover:text-ink disabled:opacity-30"
				disabled={index === 0}
				aria-label="Previous month"
				onclick={() => (index = Math.max(0, index - 1))}>‹</button
			>
			<span>{month ? `${MONTHS[month.month - 1].slice(0, 3)} ${month.year}` : '—'}</span>
			<button
				type="button"
				class="px-1 text-mute hover:text-ink disabled:opacity-30"
				disabled={index >= cal.months.length - 1}
				aria-label="Next month"
				onclick={() => (index = Math.min(cal.months.length - 1, index + 1))}>›</button
			>
		{/if}
	{/snippet}
	{#if !month}
		<p class="p-3 text-xs text-mute">—</p>
	{:else}
		<div class="p-2" data-testid="desk-calendar">
			<table class="num w-full table-fixed text-center text-2xs">
				<thead>
					<tr class="text-faint">
						{#each ['M', 'T', 'W', 'T', 'F', 'S', 'S'] as d, i (i)}<th class="py-0.5 font-normal">{d}</th>{/each}
					</tr>
				</thead>
				<tbody>
					{#each weeks as week, w (w)}
						<tr>
							{#each week as day, d (d)}
								<td class="h-7 align-top">
									{#if day.day !== null}
										<div
											class="mx-auto flex h-6 w-6 flex-col items-center justify-center {day.today ? 'border border-accent text-accent' : day.weekend ? 'text-faint' : 'text-mute'}"
											title={day.marks.map((m) => m.label).join(', ') || undefined}
										>
											<span>{day.day}</span>
											<span class="flex gap-0.5">
												{#each day.marks as m (m.kind)}<span class="size-1 rounded-full {dot[m.kind]}"></span>{/each}
											</span>
										</div>
									{/if}
								</td>
							{/each}
						</tr>
					{/each}
				</tbody>
			</table>
			<ul class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-2xs text-mute">
				{#each legend as [k, label] (k)}
					<li class="flex items-center gap-1"><span class="size-1.5 rounded-full {dot[k]}"></span>{label}</li>
				{/each}
			</ul>
			<p class="mt-2 text-2xs text-mute">
				Next FOMC:
				<span class="num text-ink">{cal?.next_fomc ? `${cal.next_fomc.date} (in ${cal.next_fomc.days} d)` : '—'}</span>
			</p>
		</div>
	{/if}
</Panel>
