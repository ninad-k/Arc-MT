<script lang="ts">
	import { desk } from '#lib/state/desk.svelte.ts';
	import { preview } from '#lib/state/preview.svelte.ts';
	import { killSwitch } from '#lib/state/killswitch.svelte.ts';
	import { num, secondsLeft } from '#lib/desk/format.ts';
	import type { PlanRecord } from '#lib/desk/types.ts';

	let now = $state(Date.now());
	$effect(() => {
		const t = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(t);
	});

	const usable = (r: PlanRecord) => r.verdict.decision === 'approved' && !r.used && r.expires_ms > now;
	const why = (r: PlanRecord) =>
		killSwitch.engaged
			? 'The kill switch is engaged'
			: preview.isOpen
				? 'Finish the open confirmation first'
				: null;
</script>

{#if desk.plans.length === 0}
	<p class="p-3 text-xs text-mute">
		No plans yet. Ask the desk to size a trade, for example "size a long on NVDA with a stop at 118". Sentinel
		approves or vetoes; Pilot only prepares a preview you confirm.
	</p>
{:else}
	<ul class="space-y-2 p-2" data-testid="plan-cards">
		{#each desk.plans as r (r.id)}
			{@const ok = r.verdict.decision === 'approved'}
			{@const left = secondsLeft(r.expires_ms, now)}
			<li class="border px-3 py-2 text-xs {ok ? 'border-up/40' : 'border-down/40'}" data-testid="plan-{r.id}">
				<div class="flex items-center justify-between gap-2">
					<span class="font-semibold tracking-wider uppercase {ok ? 'text-up' : 'text-down'}">
						{ok ? 'Approved' : 'Vetoed'}
					</span>
					<span class="num text-2xs text-faint">{r.id.slice(0, 8)}</span>
				</div>
				<p class="num mt-1 text-ink">
					{r.plan.side === 'buy' ? 'Buy' : 'Sell'} {num(r.plan.volume, 2)} lots {r.plan.symbol} @ {num(r.plan.entry, 2)}
				</p>
				<dl class="num mt-1 grid grid-cols-3 gap-1 text-2xs text-mute">
					<div><dt class="text-faint">Stop</dt><dd class="text-ink">{num(r.plan.stop, 2)}</dd></div>
					<div><dt class="text-faint">Target</dt><dd class="text-ink">{num(r.plan.target, 2)}</dd></div>
					<div>
						<dt class="text-faint">Risk</dt>
						<dd class="text-ink">{r.plan.risk_percent}% · {num(r.plan.risk_amount, 2)} {r.plan.currency ?? ''}</dd>
					</div>
				</dl>
				<ul class="mt-1 list-disc pl-4 text-2xs text-mute">
					{#each r.verdict.reasons as reason, i (i)}<li>{reason}</li>{/each}
					{#if r.verdict.decision === 'approved'}
						{#each r.verdict.warnings as w, i (i)}<li class="text-warn">{w}</li>{/each}
					{/if}
				</ul>
				{#if ok}
					<div class="mt-2 flex items-center justify-between gap-2">
						<span class="num text-2xs text-faint">
							{r.used ? 'Sent to Pilot' : left > 0 ? `valid ${left}s` : 'expired: ask Sentinel again'}
						</span>
						{#if usable(r)}
							<button
								type="button"
								class="border border-accent bg-accent/15 px-2 py-1 text-2xs font-medium tracking-wider text-accent uppercase hover:bg-accent/25 disabled:cursor-not-allowed disabled:opacity-40"
								disabled={desk.pilotBusy !== null || why(r) !== null}
								title={why(r) ?? 'Pilot prepares the preview; you confirm it in the next step'}
								onclick={() => void desk.sendToPilot(r.id)}
								data-testid="send-to-pilot"
							>
								{desk.pilotBusy === r.id ? 'Preparing…' : 'Send to Pilot'}
							</button>
						{/if}
					</div>
				{/if}
			</li>
		{/each}
	</ul>
{/if}
