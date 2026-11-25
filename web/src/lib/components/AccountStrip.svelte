<script lang="ts">
	import Panel from './Panel.svelte';
	import PollNote from './PollNote.svelte';
	import { account, alpacaAccount } from '#lib/state/hud.svelte.ts';
	import { fmtMoney, fmtPct, fmtSigned, tone } from '#lib/format.ts';

	const a = $derived(account.data);
	/** The Alpaca paper account (spec-alpaca), when Alpaca is on. */
	const al = $derived(alpacaAccount.data?.enabled ? alpacaAccount.data : null);
	const marginTone = $derived(
		!a || a.margin <= 0 ? 'flat' : a.margin_level < 100 ? 'down' : a.margin_level < 200 ? 'warn' : 'flat'
	);
</script>

<Panel title="Account" scroll={false}>
	{#snippet meta()}
		{#if a}
			<span>{a.currency}</span><span aria-hidden="true">·</span><span>1:{a.leverage}</span>
		{/if}
	{/snippet}

	<PollNote poller={account} what="account" rows={2} />

	{#if a}
		<dl class="grid grid-cols-2 gap-px bg-line/70">
			<div class="bg-panel/90 px-3 py-2.5">
				<dt class="text-2xs tracking-widest text-faint uppercase">Balance</dt>
				<dd class="num text-lg leading-tight">{fmtMoney(a.balance)}</dd>
			</div>
			<div class="bg-panel/90 px-3 py-2.5">
				<dt class="text-2xs tracking-widest text-faint uppercase">Equity</dt>
				<dd class="num text-lg leading-tight text-accent">{fmtMoney(a.equity)}</dd>
			</div>
			<div class="bg-panel/90 px-3 py-2">
				<dt class="text-2xs tracking-widest text-faint uppercase">Floating P/L</dt>
				<dd class="num text-sm t-{tone(a.profit)}">{fmtSigned(a.profit)}</dd>
			</div>
			<div class="bg-panel/90 px-3 py-2">
				<dt class="text-2xs tracking-widest text-faint uppercase">Margin level</dt>
				<dd
					class="num text-sm {marginTone === 'down'
						? 't-down'
						: marginTone === 'warn'
							? 'text-warn'
							: ''}"
				>
					{a.margin > 0 ? fmtPct(a.margin_level) : '—'}
				</dd>
			</div>
			<div class="bg-panel/90 px-3 py-2">
				<dt class="text-2xs tracking-widest text-faint uppercase">Free margin</dt>
				<dd class="num text-sm">{fmtMoney(a.free_margin)}</dd>
			</div>
			<div class="bg-panel/90 px-3 py-2">
				<dt class="text-2xs tracking-widest text-faint uppercase">Margin used</dt>
				<dd class="num text-sm text-mute">{fmtMoney(a.margin)}</dd>
			</div>
		</dl>
	{/if}
	{#if al}
		<div class="border-t border-line/70 px-3 py-2" data-testid="alpaca-account">
			<div class="flex items-center justify-between text-2xs tracking-widest text-faint uppercase">
				<span>Alpaca paper</span>
				<span class={al.market_open ? 'text-up' : 'text-mute'}>
					{al.market_open === true ? 'market open' : al.market_open === false ? 'market closed' : '—'}
				</span>
			</div>
			{#if al.error}
				<p class="mt-1 text-xs text-warn">{al.error}</p>
			{:else}
				<dl class="num mt-1 grid grid-cols-3 gap-2 text-xs">
					<div><dt class="text-faint">Equity</dt><dd class="text-ink">{al.equity === undefined ? '—' : fmtMoney(al.equity)}</dd></div>
					<div>
						<dt class="text-faint">Today</dt>
						<dd class="t-{tone(al.day_pl ?? 0)}">{al.day_pl === null || al.day_pl === undefined ? '—' : fmtSigned(al.day_pl)}</dd>
					</div>
					<div><dt class="text-faint">Buying power</dt><dd class="text-mute">{al.buying_power === null || al.buying_power === undefined ? '—' : fmtMoney(al.buying_power)}</dd></div>
				</dl>
				{#if al.blocked}<p class="mt-1 text-xs text-down">Cannot trade: {al.blocked}</p>{/if}
			{/if}
		</div>
	{/if}
</Panel>
