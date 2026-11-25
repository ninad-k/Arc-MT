<script lang="ts">
	import Panel from '#lib/components/Panel.svelte';
	import { desk } from '#lib/state/desk.svelte.ts';
	import { chartRange, priceToY } from '#lib/desk/format.ts';
	import type { LevelKind } from '#lib/desk/types.ts';

	/** Drawn exactly as Chartist sent it: no line the trader was not told about. */
	const W = 600;
	const H = 260;
	const AXIS = 54;
	const BAND_W = 40;

	const chart = $derived(desk.chart);
	const range = $derived(chart ? chartRange(chart.bars, chart.levels, chart.band) : null);
	const plotW = $derived(chart?.band ? W - AXIS - BAND_W : W - AXIS);
	const step = $derived(chart && chart.bars.length > 0 ? plotW / chart.bars.length : 0);
	const y = (p: number) => (range ? priceToY(p, range, H) : 0);
	const fmt = (p: number) => (chart ? p.toFixed(chart.digits) : String(p));

	const lineColor: Record<LevelKind, string> = {
		entry: 'var(--color-accent)',
		stop: 'var(--color-down)',
		target: 'var(--color-up)',
		supply: 'var(--color-down)',
		demand: 'var(--color-up)',
		liquidity: 'var(--color-violet)',
		marker: 'var(--color-mute)'
	};

	const ticks = $derived(
		range ? [0, 0.25, 0.5, 0.75, 1].map((t) => range.min + (range.max - range.min) * t) : []
	);
</script>

<Panel title="Desk chart" class="h-full" scroll={false}>
	{#snippet meta()}
		<span data-testid="chart-title">{chart ? `${chart.symbol} · ${chart.timeframe}` : 'Chartist'}</span>
	{/snippet}
	{#if !chart || !range || chart.bars.length === 0}
		<div class="grid flex-1 place-items-center p-4 text-center text-xs text-mute">
			Ask the desk to chart a symbol, for example "chart NVDA with the zones".
		</div>
	{:else}
		<svg viewBox="0 0 {W} {H}" class="min-h-0 w-full flex-1" preserveAspectRatio="none" role="img" aria-label="{chart.symbol} {chart.timeframe} chart with {chart.levels.length} levels" data-testid="desk-chart">
			{#each ticks as t (t)}
				<line x1="0" x2={W - AXIS} y1={y(t)} y2={y(t)} stroke="var(--color-line)" stroke-width="0.5" />
				<text x={W - AXIS + 4} y={y(t) + 3} fill="var(--color-faint)" font-size="9" font-family="var(--font-mono)">{fmt(t)}</text>
			{/each}

			<!-- Zones under the candles. -->
			{#each chart.levels.filter((l) => l.to !== null) as z, i (i)}
				<rect
					x="0"
					width={W - AXIS}
					y={y(Math.max(z.price, z.to ?? z.price))}
					height={Math.max(1, Math.abs(y(z.price) - y(z.to ?? z.price)))}
					fill={lineColor[z.kind]}
					opacity="0.12"
				/>
				<text x="4" y={y(Math.max(z.price, z.to ?? z.price)) + 10} fill={lineColor[z.kind]} font-size="9" opacity="0.9">{z.label}</text>
			{/each}

			{#each chart.bars as b, i (b.time)}
				{@const x = i * step + step / 2}
				{@const up = b.close >= b.open}
				<line x1={x} x2={x} y1={y(b.high)} y2={y(b.low)} stroke={up ? 'var(--color-up)' : 'var(--color-down)'} stroke-width="0.8" />
				<rect
					x={x - Math.max(0.6, step * 0.32)}
					width={Math.max(1.2, step * 0.64)}
					y={y(Math.max(b.open, b.close))}
					height={Math.max(0.8, Math.abs(y(b.open) - y(b.close)))}
					fill={up ? 'var(--color-up)' : 'var(--color-down)'}
				/>
			{/each}

			<!-- Lines over the candles. -->
			{#each chart.levels.filter((l) => l.to === null) as l, i (i)}
				<line x1="0" x2={W - AXIS} y1={y(l.price)} y2={y(l.price)} stroke={lineColor[l.kind]} stroke-width="1" stroke-dasharray={l.kind === 'entry' ? '' : '4 3'} />
				<text x={W - AXIS - 4} y={y(l.price) - 3} text-anchor="end" fill={lineColor[l.kind]} font-size="9">{l.label} {fmt(l.price)}</text>
			{/each}

			{#if chart.band}
				{@const bx = W - AXIS - BAND_W}
				<rect x={bx + 4} width={BAND_W - 8} y={y(chart.band.high)} height={Math.max(1, y(chart.band.low) - y(chart.band.high))} fill="var(--color-violet)" opacity="0.25" />
				<title>{chart.band.label}</title>
			{/if}
		</svg>
		{#if chart.band}
			<p class="px-3 pb-1 text-2xs text-violet">{chart.band.label}: {fmt(chart.band.low)} to {fmt(chart.band.high)}</p>
		{/if}
	{/if}
</Panel>
