<script lang="ts">
	import type { MarketViewDto } from '#lib/api.ts';
	import { fmtPrice } from '#lib/format.ts';
	import {
		biasInfo,
		clampConfidence,
		clockTime,
		confidenceWord,
		eventMinutes,
		expiryText,
		fmtLevel,
		fmtPoints,
		importanceInfo,
		isExpired,
		macdSign,
		minutesAwayText,
		newsLabel,
		newsTone,
		rsiTone,
		safeHttpUrl,
		timeAgo,
		toneClass,
		topHeadlines,
		trendArrow,
		trendWord,
		viewDigits
	} from '#lib/market.ts';
	import { symbolLabel } from '#lib/symbols.ts';

	let {
		view,
		now,
		digitsHint = null,
		preparable,
		blockedReason = null,
		onprepare
	}: {
		view: MarketViewDto;
		now: number;
		/** The watchlist's digits for this instrument, when it has one. */
		digitsHint?: number | null;
		/** The view has a setup and has not run out. */
		preparable: boolean;
		/** Why trading actions are off right now (kill switch, bridge down), or null. */
		blockedReason?: string | null;
		onprepare: () => void;
	} = $props();

	const d = $derived(viewDigits(view, digitsHint));
	const bias = $derived(biasInfo(view.bias));
	const confidence = $derived(clampConfidence(view.confidence));
	const label = $derived(symbolLabel({ symbol: view.broker_symbol, canonical: view.symbol }));
	const expired = $derived(isExpired(view.valid_until, now));
	const headlines = $derived(topHeadlines(view.news.headlines, 5));
	const lv = $derived(view.levels);

	const levelRows = $derived([
		{ k: 'Pivot', v: lv.pivot },
		{ k: 'R1', v: lv.r1 },
		{ k: 'R2', v: lv.r2 },
		{ k: 'S1', v: lv.s1 },
		{ k: 'S2', v: lv.s2 },
		{ k: 'Prev. high', v: lv.prev_day_high },
		{ k: 'Prev. low', v: lv.prev_day_low },
		{ k: 'VWAP', v: lv.vwap }
	]);

	const h = 'hud-title mb-1.5';
</script>

<article class="space-y-4 p-3" data-testid="market-card" data-symbol={view.symbol} data-bias={view.bias} aria-label="{view.symbol} market view">
	<!-- Header: instrument, price, bias, confidence -->
	<header class="space-y-2.5">
		<div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
			<h3 class="flex min-w-0 items-baseline gap-2 text-base font-semibold">
				<span class="truncate" data-testid="market-symbol">{label.primary}</span>
				{#if label.secondary}<span class="num truncate text-2xs font-normal text-mute" title="Broker symbol">{label.secondary}</span>{/if}
			</h3>
			<p class="num text-xs text-mute">
				<span class="sr-only">Bid </span>{fmtPrice(view.price.bid, d)}
				<span aria-hidden="true">/</span><span class="sr-only"> ask </span>
				{fmtPrice(view.price.ask, d)}
			</p>
		</div>

		<div class="flex flex-wrap items-center gap-x-3 gap-y-2">
			<span class="pill h-7 px-2.5 text-xs font-semibold {bias.pill}" data-testid="bias-pill">
				<span aria-hidden="true">{trendArrow(view.bias)}</span>
				{bias.label}
			</span>
			<div class="min-w-32 flex-1">
				<div class="flex items-baseline justify-between gap-2 text-2xs text-mute">
					<span>Confidence</span>
					<span class="num" data-testid="confidence-text">{confidence} / 100 · {confidenceWord(confidence)}</span>
				</div>
				<div
					class="mt-1 h-1.5 w-full bg-raise"
					role="meter"
					aria-label="Confidence"
					aria-valuemin="0"
					aria-valuemax="100"
					aria-valuenow={confidence}
					aria-valuetext="{confidence} of 100, {confidenceWord(confidence)}"
				>
					<div class="h-full {bias.bar}" style="width:{confidence}%"></div>
				</div>
			</div>
		</div>
	</header>

	{#if view.stand_aside}
		<div class="border border-warn/70 bg-warn/10 px-3 py-2 text-warn" data-testid="stand-aside">
			<p class="text-sm font-semibold"><span aria-hidden="true">△ </span>Stand aside</p>
			<p class="mt-0.5 text-xs">{view.stand_aside_reason ?? 'Conditions are not suitable for a trade right now.'}</p>
		</div>
	{/if}

	<p class="text-sm leading-snug" data-testid="market-summary">{view.summary}</p>

	<!-- Setup -->
	{#if view.setup}
		{@const s = view.setup}
		<section class="@container border border-line-hi bg-panel/70 p-3" aria-labelledby="mv-setup-title" data-testid="market-setup">
			<div class="mb-2 flex items-center justify-between gap-2">
				<h4 id="mv-setup-title" class="hud-title">Setup</h4>
				<span class="pill h-5 px-1.5 {s.side === 'buy' ? 't-up' : 't-down'}">{s.side}</span>
			</div>
			<dl class="grid grid-cols-2 gap-x-4 gap-y-2 text-xs @md:grid-cols-3">
				<div class="col-span-2 @md:col-span-1">
					<dt class="text-mute">Entry zone</dt>
					<dd class="num text-sm font-medium">{fmtPrice(s.entry_low, d)} – {fmtPrice(s.entry_high, d)}</dd>
				</div>
				<div>
					<dt class="text-mute">Stop loss</dt>
					<dd class="num text-sm font-medium t-down">{fmtPrice(s.stop_loss, d)}</dd>
				</div>
				<div>
					<dt class="text-mute">TP1</dt>
					<dd class="num text-sm font-medium t-up">{fmtPrice(s.take_profit, d)}</dd>
				</div>
				<div>
					<dt class="text-mute">TP2</dt>
					<dd class="num text-sm font-medium t-up">{s.take_profit_2 === null ? '—' : fmtPrice(s.take_profit_2, d)}</dd>
				</div>
				<div>
					<dt class="text-mute">Reward : risk</dt>
					<dd class="num text-sm font-medium">{s.risk_reward.toFixed(2)}</dd>
				</div>
				<div class="col-span-2 @md:col-span-1">
					<dt class="text-mute">ATR H1 / M15</dt>
					<dd class="num text-sm">{fmtPrice(s.atr_h1, d)} / {fmtPrice(s.atr_m15, d)}</dd>
				</div>
			</dl>
			<div class="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-line pt-3">
				{#if preparable}
					<button
						type="button"
						class="btn btn-primary h-10"
						disabled={blockedReason !== null}
						title={blockedReason ?? 'Turn this setup into a preview you can confirm'}
						onclick={onprepare}
					>
						Prepare trade
					</button>
					<p class="min-w-0 flex-1 basis-40 text-2xs text-mute">
						{blockedReason ?? 'Opens a preview only. Nothing is sent until you confirm it.'}
					</p>
				{:else if expired}
					<p class="text-xs text-warn" data-testid="setup-expired">
						<span aria-hidden="true">△ </span>This view has expired. Analyze again for a current setup.
					</p>
				{/if}
			</div>
		</section>
	{:else if !view.stand_aside}
		<p class="border border-line bg-panel/50 px-3 py-2 text-xs text-mute" data-testid="no-setup">
			No trade setup right now. The bias is {bias.label.toLowerCase()} but not strong or clear enough to size a stop and targets.
		</p>
	{/if}

	<p class="num text-2xs {expired ? 'text-warn' : 'text-mute'}" data-testid="valid-until">
		Made {clockTime(view.generated_at)} · {expiryText(view.valid_until, now)}
	</p>

	<!-- Timeframes -->
	<section aria-labelledby="mv-tf-title">
		<h4 id="mv-tf-title" class={h}>Timeframes</h4>
		<div class="border border-line">
			<table class="w-full text-xs">
				<caption class="sr-only">Trend, RSI and MACD histogram per timeframe</caption>
				<thead class="text-2xs tracking-widest text-mute uppercase">
					<tr class="border-b border-line">
						<th scope="col" class="px-2.5 py-1.5 text-left font-medium">TF</th>
						<th scope="col" class="px-2.5 py-1.5 text-left font-medium">Trend</th>
						<th scope="col" class="px-2.5 py-1.5 text-right font-medium">RSI</th>
						<th scope="col" class="px-2.5 py-1.5 text-right font-medium">MACD</th>
					</tr>
				</thead>
				<tbody>
					{#each view.timeframes as t (t.timeframe)}
						{@const m = macdSign(t.macd_hist)}
						<tr class="border-t border-line/60" data-testid="tf-row">
							<th scope="row" class="num px-2.5 py-1.5 text-left font-medium">{t.timeframe}</th>
							<td class="px-2.5 py-1.5 {toneClass(t.trend === 'up' ? 'up' : t.trend === 'down' ? 'down' : 'flat')}">
								<span aria-hidden="true">{trendArrow(t.trend)}</span>
								<span class="sr-only">{trendWord(t.trend)}</span>
							</td>
							<td class="num px-2.5 py-1.5 text-right {rsiTone(t.rsi) === 'warn' ? 'text-warn' : ''}">
								{t.rsi.toFixed(1)}{#if rsiTone(t.rsi) === 'warn'}<span class="sr-only"> (stretched)</span>{/if}
							</td>
							<td class="num px-2.5 py-1.5 text-right {toneClass(m.tone)}">
								<span aria-hidden="true">{m.text}</span><span class="sr-only">{m.word}</span>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	</section>

	<!-- Key levels -->
	<section aria-labelledby="mv-lv-title">
		<h4 id="mv-lv-title" class={h}>Key levels</h4>
		<dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-xs sm:grid-cols-4">
			{#each levelRows as r (r.k)}
				<div class="flex items-baseline justify-between gap-2 border-b border-line/50 py-1">
					<dt class="text-mute">{r.k}</dt>
					<dd class="num">{fmtLevel(r.v, d)}</dd>
				</div>
			{/each}
		</dl>
		{#if lv.support.length || lv.resistance.length}
			<dl class="mt-2 grid gap-y-1 text-xs">
				{#if lv.resistance.length}
					<div class="flex flex-wrap items-baseline gap-x-2">
						<dt class="w-20 shrink-0 text-mute">Resistance</dt>
						<dd class="num t-down">{lv.resistance.map((v) => fmtPrice(v, d)).join(' · ')}</dd>
					</div>
				{/if}
				{#if lv.support.length}
					<div class="flex flex-wrap items-baseline gap-x-2">
						<dt class="w-20 shrink-0 text-mute">Support</dt>
						<dd class="num t-up">{lv.support.map((v) => fmtPrice(v, d)).join(' · ')}</dd>
					</div>
				{/if}
			</dl>
		{/if}
	</section>

	<!-- Events -->
	<section aria-labelledby="mv-ev-title">
		<h4 id="mv-ev-title" class={h}>Upcoming events</h4>
		{#if view.events.length === 0}
			<p class="text-xs text-mute">No relevant events in the next hours.</p>
		{:else}
			<ul class="divide-y divide-line/60 border border-line text-xs" data-testid="market-events">
				{#each view.events as e, i (`${e.time}#${i}`)}
					{@const imp = importanceInfo(e.importance)}
					{@const mins = eventMinutes(e, now)}
					<li class="flex flex-wrap items-center gap-x-2 gap-y-0.5 px-2.5 py-1.5" data-testid="event-row">
						<span class="num w-24 shrink-0 text-mute">{minutesAwayText(mins)}</span>
						<span class="num border border-line-hi px-1 text-2xs">{e.currency}</span>
						<span class="min-w-0 flex-1 basis-32 break-words">{e.title}</span>
						<span class="pill h-5 px-1.5 {imp.cls}">{imp.label}<span class="sr-only"> impact</span></span>
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<!-- News -->
	<section aria-labelledby="mv-news-title" data-testid="market-news">
		<h4 id="mv-news-title" class={h}>News tone</h4>
		{#if !view.news.configured}
			<p class="text-xs text-mute">
				<span class="font-medium text-ink">Not configured.</span> No news provider is set up, so this view uses prices and the calendar only.
			</p>
		{:else}
			<p class="text-sm font-medium {toneClass(newsTone(view.news.score))}" data-testid="news-label">{newsLabel(view.news)}</p>
			{#if headlines.length}
				<ul class="mt-2 divide-y divide-line/60 border border-line text-xs">
					{#each headlines as n, i (`${n.url}#${i}`)}
						{@const href = safeHttpUrl(n.url)}
						<li class="px-2.5 py-1.5">
							{#if href}
								<a
									{href}
									target="_blank"
									rel="noopener noreferrer"
									class="break-words text-ink underline decoration-line-hi underline-offset-2 hover:text-accent"
								>
									{n.title}<span class="sr-only"> (opens in a new tab)</span>
								</a>
							{:else}
								<span class="break-words">{n.title}</span>
							{/if}
							<p class="mt-0.5 flex flex-wrap gap-x-2 text-2xs text-mute">
								<span>{n.source}</span>
								<span aria-hidden="true">·</span>
								<span>{timeAgo(n.published_at, now)}</span>
								<span aria-hidden="true">·</span>
								<span class={toneClass(newsTone(n.tone))}>tone {n.tone > 0 ? '+' : ''}{n.tone.toFixed(2)}</span>
							</p>
						</li>
					{/each}
				</ul>
			{/if}
		{/if}
	</section>

	<!-- Factors -->
	<section aria-labelledby="mv-fa-title">
		<h4 id="mv-fa-title" class={h}>Why <span class="num normal-case">· score {fmtPoints(view.score)}</span></h4>
		<ul class="divide-y divide-line/60 border border-line text-xs" data-testid="market-factors">
			{#each view.factors as f (f.name)}
				{@const tone = f.points > 0 ? 'up' : f.points < 0 ? 'down' : 'flat'}
				<li class="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-3 px-2.5 py-1.5" data-testid="factor-row">
					<span class="min-w-0">
						<span class="font-medium">{f.name}</span>
						<span class="block text-2xs text-mute">{f.detail}</span>
					</span>
					<span class="num font-medium {toneClass(tone)}">{fmtPoints(f.points)}<span class="sr-only"> points, {f.signal}</span></span>
				</li>
			{/each}
		</ul>
	</section>
</article>
