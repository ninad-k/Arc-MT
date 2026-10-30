<script lang="ts">
	import { decimalsOf, fmtPrice } from '#lib/format.ts';
	import { biasInfo, biasVerdict, fmtPercent, outcomeInfo, summarizeStats, timeAgo, trendArrow } from '#lib/market.ts';
	import { digitsFor } from '#lib/state/hud.svelte.ts';
	import { market } from '#lib/state/market.svelte.ts';

	const phase = $derived(market.journalPhase);
	const stats = $derived(summarizeStats(market.journal?.stats));
	const rows = $derived(market.journal?.views ?? []);

	/** Journal rows carry no digits: use the watchlist's, else the number's own decimals. */
	function price(v: number, brokerSymbol: string | undefined): string {
		const d = (brokerSymbol ? digitsFor(brokerSymbol) : undefined) ?? Math.max(decimalsOf(v), v >= 100 ? 2 : 4);
		return fmtPrice(v, d);
	}

	const tiles = $derived([
		{
			id: 'setups',
			label: 'Setups',
			value: String(stats.setups),
			sub: `${stats.settled} settled · ${stats.open} open`
		},
		{
			id: 'tp',
			label: 'TP hit rate',
			value: fmtPercent(stats.tpRate),
			sub: stats.settled ? `${stats.tpHit} of ${stats.settled}` : 'no settled setups yet',
			cls: 't-up'
		},
		{
			id: 'sl',
			label: 'SL hit',
			value: fmtPercent(stats.slRate),
			sub: stats.settled ? `${stats.slHit} of ${stats.settled}` : 'no settled setups yet',
			cls: 't-down'
		},
		{
			id: 'expired',
			label: 'Expired',
			value: fmtPercent(stats.expiredRate),
			sub: stats.settled ? `${stats.expired} of ${stats.settled}` : 'no settled setups yet',
			cls: 'text-warn'
		},
		{
			id: 'bias',
			label: 'Bias accuracy',
			value: fmtPercent(stats.biasAccuracy),
			sub: stats.biasScored ? `${stats.biasCorrect} of ${stats.biasScored} scored` : 'nothing scored yet'
		}
	]);
</script>

<div class="space-y-4 p-3" data-testid="track-record">
	{#if phase === 'loading' || phase === 'idle'}
		<div class="space-y-2" aria-busy="true" aria-label="Loading the track record">
			<div class="skeleton h-16"></div>
			<div class="skeleton h-9"></div>
			<div class="skeleton h-9"></div>
		</div>
	{:else if phase === 'unsupported'}
		<p class="text-xs text-mute" role="status">This server does not keep a track record yet.</p>
	{:else if phase === 'error'}
		<div class="flex flex-col items-start gap-2 text-xs" role="alert">
			<p class="text-warn"><span aria-hidden="true">△ </span>Could not load the track record. {market.journalError ?? ''}</p>
			<button type="button" class="btn" onclick={() => void market.loadJournal()}>Retry</button>
		</div>
	{:else}
		<p class="text-xs text-mute">
			Every view is stored and later compared with what the price did. Rates are shares of the settled setups.
		</p>

		<dl class="grid grid-cols-2 gap-2 sm:grid-cols-3" data-testid="track-stats">
			{#each tiles as t (t.id)}
				<div class="border border-line bg-panel px-2.5 py-2" data-testid="stat-{t.id}">
					<dt class="text-2xs tracking-widest text-mute uppercase">{t.label}</dt>
					<dd class="num text-lg font-semibold {t.cls ?? ''}">{t.value}</dd>
					<dd class="num text-2xs text-mute">{t.sub}</dd>
				</div>
			{/each}
		</dl>

		<section aria-labelledby="mv-latest-title">
			<div class="mb-1.5 flex items-center justify-between gap-2">
				<h4 id="mv-latest-title" class="hud-title">Latest views</h4>
				<button type="button" class="btn h-7 px-2.5" onclick={() => void market.loadJournal()}>Refresh</button>
			</div>
			{#if market.journalError}
				<p class="mb-2 text-xs text-warn" role="status"><span aria-hidden="true">△ </span>Could not refresh: {market.journalError}</p>
			{/if}
			{#if rows.length === 0}
				<p class="border border-line bg-panel/50 px-3 py-3 text-xs text-mute" role="status">
					No views yet. Analyze a symbol and it will be listed here once made.
				</p>
			{:else}
				<ul class="divide-y divide-line/60 border border-line" data-testid="journal-list">
					{#each rows as r (r.id)}
						{@const b = biasInfo(r.bias)}
						{@const o = outcomeInfo(r.outcome)}
						<li class="space-y-1 px-2.5 py-2 text-xs" data-testid="journal-row" data-outcome={r.outcome}>
							<div class="flex flex-wrap items-center gap-x-2 gap-y-1">
								<span class="font-medium">{r.symbol}</span>
								<span class="pill h-5 px-1.5 {b.pill}"><span aria-hidden="true">{trendArrow(r.bias)}</span> {b.label}</span>
								<span class="num text-mute">{r.confidence}<span class="sr-only"> of 100 confidence</span></span>
								<span class="pill ml-auto h-5 px-1.5 {o.cls}" data-testid="outcome-badge">{o.label}</span>
							</div>
							<p class="text-2xs text-mute">
								{timeAgo(r.created_at, market.now)}
								{#if r.side && r.entry !== null}
									· <span class="num">{r.side} @ {price(r.entry, r.broker_symbol)}{#if r.stop !== null}, SL {price(r.stop, r.broker_symbol)}{/if}{#if r.tp !== null}, TP {price(r.tp, r.broker_symbol)}{/if}</span>
								{:else if r.stand_aside}
									· stand aside
								{/if}
								· <span class={r.bias_correct === true ? 't-up' : r.bias_correct === false ? 't-down' : ''}>{biasVerdict(r.bias_correct)}</span>
							</p>
						</li>
					{/each}
				</ul>
			{/if}
		</section>
	{/if}
</div>
