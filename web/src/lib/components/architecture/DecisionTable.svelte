<script lang="ts">
	import { architecture } from '#lib/state/architecture.svelte.ts';
	import { STAGES, STAGE_LABEL, clock, filterQuery, rClass, rText, resultText, signed } from '#lib/architecture/format.ts';
	import StageStrip from './StageStrip.svelte';

	let { selectedId, onselect }: { selectedId: string | null; onselect: (id: string) => void } = $props();

	const rows = $derived(architecture.visible);
	const exportHref = (format: 'csv' | 'full') =>
		`/api/architecture/decisions?${filterQuery(
			{ ...architecture.rangeQuery(), symbol: architecture.filter.symbol, status: architecture.filter.status as never, stage: architecture.filter.stage as never, stop: architecture.filter.stop },
			{ format }
		)}`;

	const reload = () => void architecture.loadDecisions();
	function onkey(e: KeyboardEvent, id: string) {
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			onselect(id);
		}
	}
	const field =
		'num h-7 border border-line bg-panel/70 px-2 text-2xs text-ink focus-visible:border-accent';
</script>

<div data-testid="decision-table">
	<div class="mb-2 flex flex-wrap items-end gap-x-3 gap-y-2">
		<label class="grid gap-0.5 text-2xs text-faint">
			Range
			<select class={field} bind:value={architecture.range} onchange={reload}>
				<option value="24h">Last 24 h</option>
				<option value="7d">Last 7 days</option>
				<option value="30d">Last 30 days</option>
			</select>
		</label>
		<label class="grid gap-0.5 text-2xs text-faint">
			Symbol
			<input
				class="{field} w-24 uppercase"
				placeholder="All"
				bind:value={architecture.filter.symbol}
				onchange={reload}
				data-testid="filter-symbol"
			/>
		</label>
		<label class="grid gap-0.5 text-2xs text-faint">
			Status
			<select class={field} bind:value={architecture.filter.status} onchange={reload} data-testid="filter-status">
				<option value="">All</option>
				<option value="traded">Traded</option>
				<option value="stopped">Stopped</option>
				<option value="waiting">Waiting</option>
				<option value="open">Open</option>
			</select>
		</label>
		<label class="grid gap-0.5 text-2xs text-faint">
			Stage
			<select class={field} bind:value={architecture.filter.stage} onchange={reload} data-testid="filter-stage">
				<option value="">All</option>
				{#each STAGES as s (s)}<option value={s}>{STAGE_LABEL[s]}</option>{/each}
			</select>
		</label>
		<label class="grid gap-0.5 text-2xs text-faint">
			Stop code
			<select class="{field} max-w-48" bind:value={architecture.filter.stop} onchange={reload} data-testid="filter-stop">
				<option value="">Any</option>
				{#each architecture.knownStops as c (c)}<option value={c}>{c}</option>{/each}
			</select>
		</label>
		{#if architecture.minStep !== null}
			<button type="button" class="pill border-accent text-accent" onclick={() => (architecture.minStep = null)}>
				Reached step {architecture.minStep}+ ✕
			</button>
		{/if}
		<span class="ml-auto flex gap-2">
			<a class="btn h-7" href={exportHref('csv')} download="decisions.csv" data-testid="export-csv">Export CSV</a>
			<a class="btn h-7" href={exportHref('full')} download="decisions.json" data-testid="export-json">Export JSON</a>
		</span>
	</div>

	{#if architecture.listError}
		<div class="mb-2 flex items-center justify-between gap-3 border border-down/60 bg-down/10 px-3 py-2 text-xs text-down" role="alert">
			<span>{architecture.listError}</span>
			<button type="button" class="underline" onclick={reload}>Retry</button>
		</div>
	{/if}

	<div class="overflow-x-auto">
		<table class="w-full border-collapse text-xs">
			<thead>
				<tr class="text-left text-[10.5px] tracking-wider text-faint uppercase">
					<th scope="col" class="px-2 py-1.5 font-medium">Time</th>
					<th scope="col" class="px-2 py-1.5 font-medium">Symbol</th>
					<th scope="col" class="px-2 py-1.5 font-medium max-md:hidden">Trigger</th>
					<th scope="col" class="px-2 py-1.5 font-medium">Stages</th>
					<th scope="col" class="px-2 py-1.5 font-medium">Score</th>
					<th scope="col" class="px-2 py-1.5 font-medium">Result / why not</th>
					<th scope="col" class="px-2 py-1.5 font-medium max-md:hidden">R</th>
					<th scope="col" class="px-2 py-1.5 font-medium max-lg:hidden">Tags</th>
				</tr>
			</thead>
			<tbody>
				{#each rows as d (d.id)}
					{@const res = resultText(d)}
					<tr
						class="cursor-pointer border-t border-line/60 hover:bg-raise focus-visible:bg-raise {selectedId === d.id ? 'bg-accent/10' : ''}"
						tabindex="0"
						aria-selected={selectedId === d.id}
						aria-label="{d.symbol} {clock(d.created_ms)}: {res.text}"
						data-testid="decision-row"
						data-id={d.id}
						onclick={() => onselect(d.id)}
						onkeydown={(e) => onkey(e, d.id)}
					>
						<td class="num px-2 py-1.5 whitespace-nowrap text-mute">{clock(d.created_ms)}</td>
						<td class="px-2 py-1.5 whitespace-nowrap">
							<b>{d.symbol}</b> <span class="text-faint">{d.venue === 'alpaca' ? 'Alpaca' : 'MT5'}</span>
						</td>
						<td class="max-w-44 truncate px-2 py-1.5 text-mute max-md:hidden">{d.trigger.detail ?? d.trigger.kind}</td>
						<td class="px-2 py-1.5"><StageStrip step={d.step} status={d.status} /></td>
						<td class="num px-2 py-1.5">{signed(d.score)}</td>
						<td class="px-2 py-1.5 {res.cls}" data-testid="decision-result">
							{res.text}
							{#if d.stop_code}<span class="num ml-1 text-2xs text-faint max-xl:hidden">{d.stop_code}</span>{/if}
						</td>
						<td class="num px-2 py-1.5 whitespace-nowrap max-md:hidden {rClass(d.result_r)}">{rText(d.result_r)}</td>
						<td class="px-2 py-1.5 text-faint max-lg:hidden">{d.tags.join(' · ') || '—'}</td>
					</tr>
				{:else}
					<tr>
						<td colspan="8" class="p-6 text-center text-mute">
							<span role="status">{architecture.listLoading ? 'Loading decisions…' : 'No decisions match. Click Scan now to look at the watchlist.'}</span>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
</div>
