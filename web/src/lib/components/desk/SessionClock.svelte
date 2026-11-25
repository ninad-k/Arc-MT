<script lang="ts">
	import Panel from '#lib/components/Panel.svelte';
	import { desk } from '#lib/state/desk.svelte.ts';
	import { countdown, phaseTone } from '#lib/desk/format.ts';

	let now = $state(Date.now());
	$effect(() => {
		const t = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(t);
	});

	const s = $derived(desk.state?.session ?? null);
	const toNext = $derived(s?.next_change ? (Date.parse(s.next_change) - now) / 1000 : null);
	const nextLabel: Record<string, string> = {
		pre_market: 'Pre-market opens',
		open: 'Market opens',
		after_hours: 'Market closes',
		closed: 'After-hours end'
	};
	const tone = $derived(phaseTone(s?.phase));
</script>

<Panel title="US session">
	{#snippet meta()}
		<span>NYSE</span>
	{/snippet}
	<div class="p-3" data-testid="session-clock">
		<div class="flex items-baseline justify-between gap-2">
			<span
				class="text-lg font-semibold tracking-[0.18em] uppercase {tone === 'up' ? 'text-up' : tone === 'warn' ? 'text-warn' : 'text-mute'}"
				data-testid="session-phase"
			>
				{s?.label ?? '—'}
			</span>
			<span class="num text-sm text-ink">{s ? `${s.ny_time} ${s.ny_zone}` : '—'}</span>
		</div>
		<p class="num mt-1 text-2xs text-mute">{s?.ny_date ?? '—'}{s?.early_close ? ' · early close 13:00' : ''}{s && !s.trading_day ? ' · no trading today' : ''}</p>
		<div class="mt-3 border-t border-line/70 pt-2 text-xs">
			<div class="flex justify-between">
				<span class="text-mute">{s?.next_phase ? nextLabel[s.next_phase] : 'Next change'}</span>
				<span class="num text-ink">{countdown(toNext)}</span>
			</div>
			{#if s?.phase === 'open'}
				{@const toClose = s.seconds_to_close === null ? null : s.seconds_to_close - (now - desk.loadedAt) / 1000}
				<div class="mt-1 flex justify-between">
					<span class="text-mute">To the close</span>
					<span class="num text-ink">{countdown(toClose)}</span>
				</div>
			{/if}
		</div>
	</div>
</Panel>
