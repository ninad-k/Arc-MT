<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '#lib/api.ts';
	import { architecture } from '#lib/state/architecture.svelte.ts';
	import { DASH, clock, signed, stanceBg, stanceChip, stanceClass } from '#lib/architecture/format.ts';
	import type { ArchitectureMap, DecisionDetail, MapMember, MemberId, OpinionRecord } from '#lib/architecture/types.ts';

	let {
		nodeId,
		map,
		latest,
		onclose
	}: { nodeId: string; map: ArchitectureMap; latest: DecisionDetail | null; onclose: () => void } = $props();

	let panel = $state<HTMLElement>();
	let closeBtn = $state<HTMLButtonElement>();
	const uid = $props.id();

	// v1 mapper rules (crates/desk/src/opinion.rs).
	const RULES: Record<MemberId, { role: string; how: string; inputs: string }> = {
		oracle: {
			role: 'General member · probabilities',
			how: 'P(up) ≥ 0.55 is bull and ≤ 0.45 is bear. Confidence = |P(up) − 0.5| × 200.',
			inputs: 'Empirical returns over the horizon. No LLM.'
		},
		scout: {
			role: 'General member · momentum',
			how: "Today's direction. Confidence comes from how much of the day's range the move covers, plus up to 20 for relative volume.",
			inputs: 'Bars and movers from the broker feed. No LLM.'
		},
		chartist: {
			role: 'General member · levels and band',
			how: 'Room to the nearest level above versus below the price (supply, demand, liquidity, targets, stops). A ratio ≥ 1.5 is bull and ≤ 1/1.5 is bear.',
			inputs: 'Bars and the levels Chartist draws. No LLM.'
		},
		athena: {
			role: 'General member · supply / demand and fundamentals',
			how: 'The price at or inside a supply zone (within a quarter ATR) is bear, at a demand zone bull. Both or neither is neutral. Fundamentals are shown as evidence only.',
			inputs: 'Zones from bars, fundamentals from Finnhub. No LLM.'
		},
		atlas: {
			role: 'General member · macro',
			how: "Risk tone of the headlines and the indices' moves. Stocks and indices only.",
			inputs: 'Finnhub news and the economic calendar. No LLM.'
		},
		capitol: {
			role: 'General member · insiders and Congress',
			how: 'Insider open-market buys versus sales, stocks only. Sales are often routine, so a bearish vote counts half.',
			inputs: 'Finnhub insider and Congress filings. No LLM.'
		}
	};
	const COMMON = 'A member with no usable data votes neutral with confidence 0 and says why in its evidence. Mapper v1: free, deterministic, versioned.';

	const STATIC: Record<string, { title: string; role: string; body: string[] }> = {
		sig_scout: { title: 'Scout movers', role: 'Signal', body: ['Starts a decision when a watchlist symbol is among the biggest movers.', 'Free: runs in the council scan every few minutes during the US session.'] },
		sig_view: { title: 'Market view', role: 'Signal', body: ['A market view (bias plus setup) on a symbol can start a decision.', 'Existing market-view data is reused; no new paid feed.'] },
		sig_calendar: { title: 'Calendar & tone', role: 'Signal', body: ['Atlas calendar or tone events (FOMC, earnings, headlines) can start a decision, and can also block one (event blackout).'] },
		sig_question: { title: 'Your question', role: 'Signal', body: ['A question you ask in Desk mode starts a decision with the trigger "question".'] },
		bull_case: { title: 'Bull case', role: 'Council seat', body: ["The strongest weighted evidence for the long side, assembled in code from the members' opinions. No LLM."] },
		bear_case: { title: 'Bear case', role: 'Council seat', body: ["The strongest weighted evidence for the short side, assembled in code from the members' opinions. No LLM."] },
		chair: {
			title: 'Chair',
			role: 'Council seat',
			body: [
				'Score = Σ weight × sign(stance) × confidence / 100, from −1 to +1.',
				'The council passes when |score| ≥ min_council_score and enough members are non-neutral.'
			]
		},
		pilot: { title: 'Pilot', role: 'Execution', body: ['Turns an approved plan into a preview and re-checks Sentinel. It only prepares: nothing executes without your confirm.'] },
		gate: { title: 'Gate', role: 'Execution', body: ['The [limits] section (max lot, positions, daily loss, allowed symbols, hours), the kill switch, and the account lock. The same gate every order passes.'] },
		confirm: { title: 'Your confirm', role: 'Execution', body: ['A 15 second token. Nothing executes without it, and a preview that expires is a stop (preview_expired).'] },
		broker: { title: 'Broker', role: 'Execution', body: ['MT5 or Alpaca paper. The fill, or the broker refusal, ends the decision.'] }
	};

	const member = $derived(map.members.find((m) => m.agent === nodeId) as MapMember | undefined);
	const rules = $derived(member ? RULES[member.agent] : null);
	const info = $derived(STATIC[nodeId] ?? null);
	const title = $derived(
		member ? member.agent.charAt(0).toUpperCase() + member.agent.slice(1) : nodeId === 'friday' ? 'Friday' : nodeId === 'sentinel' ? 'Sentinel' : (info?.title ?? nodeId)
	);
	const role = $derived(
		member ? (rules?.role ?? '') : nodeId === 'friday' ? 'Master · plans the trade' : nodeId === 'sentinel' ? 'Council · risk seat (veto)' : (info?.role ?? '')
	);
	const dotCls = $derived(member?.latest ? stanceBg(member.latest.stance) : nodeId === 'friday' ? 'bg-violet' : 'bg-faint');

	const guards = $derived(map.live.guards);
	const GUARD_LABEL: [keyof typeof guards, string][] = [
		['risk_percent', 'risk_percent (%)'],
		['max_open_positions', 'max_open_positions'],
		['no_new_trades_before_close_minutes', 'no new trades before close (min)'],
		['min_reward_risk', 'min_reward_risk'],
		['min_council_score', 'min_council_score'],
		['min_non_neutral', 'min non-neutral members']
	];

	// Recent opinions of this member, read from the newest decisions.
	let recent = $state<{ d: { id: string; symbol: string; created_ms: number }; o: OpinionRecord }[]>([]);
	$effect(() => {
		const id = nodeId as MemberId;
		if (!member) {
			recent = [];
			return;
		}
		let cancelled = false;
		void (async () => {
			let list = architecture.decisions;
			if (list.length === 0) {
				try {
					list = (await api.architectureDecisions({ limit: 8 })).decisions;
				} catch {
					list = [];
				}
			}
			const got = await Promise.allSettled(list.slice(0, 8).map((d) => api.architectureDecision(d.id)));
			if (cancelled) return;
			recent = got
				.flatMap((r) => (r.status === 'fulfilled' ? [r.value] : []))
				.flatMap((d) => {
					const o = d.opinions.find((x) => x.agent === id);
					return o ? [{ d: { id: d.id, symbol: d.symbol, created_ms: d.created_ms }, o }] : [];
				})
				.slice(0, 6);
		})();
		return () => {
			cancelled = true;
		};
	});

	// Dialog behaviour: focus in, trap Tab, Esc closes, focus returns to the node.
	onMount(() => {
		const before = document.activeElement as HTMLElement | null;
		closeBtn?.focus();
		return () => before?.focus?.();
	});

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.stopPropagation();
			onclose();
			return;
		}
		if (e.key !== 'Tab' || !panel) return;
		const f = [...panel.querySelectorAll<HTMLElement>('a[href],button:not([disabled]),input,select,textarea,summary,[tabindex]:not([tabindex="-1"])')].filter((x) => x.offsetParent !== null);
		if (f.length === 0) return;
		const first = f[0];
		const last = f[f.length - 1];
		if (e.shiftKey && document.activeElement === first) {
			e.preventDefault();
			last.focus();
		} else if (!e.shiftKey && document.activeElement === last) {
			e.preventDefault();
			first.focus();
		}
	}
</script>

<svelte:window {onkeydown} />

<div class="fixed inset-0 z-40 bg-void/60" aria-hidden="true" onclick={onclose} data-testid="drawer-scrim"></div>
<div
	bind:this={panel}
	class="fixed top-0 right-0 bottom-0 z-50 flex w-[min(440px,100%)] flex-col border-l border-line-hi bg-panel"
	role="dialog"
	aria-modal="true"
	aria-labelledby="{uid}-title"
	data-testid="node-drawer"
>
	<div class="flex items-center gap-3 border-b border-line px-4 py-3">
		<span class="size-2.5 rounded-full {dotCls}" aria-hidden="true"></span>
		<div class="min-w-0">
			<h2 id="{uid}-title" class="font-semibold">{title}</h2>
			<p class="text-[11.5px] text-faint">{role}</p>
		</div>
		<button bind:this={closeBtn} type="button" class="btn ml-auto h-8" aria-label="Close details" onclick={onclose} data-testid="drawer-close">✕</button>
	</div>

	<div class="grid gap-3 overflow-y-auto p-4 text-xs">
		{#if member && rules}
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">Newest opinion</h3>
				{#if member.latest}
					<p class="num text-base {stanceClass(member.latest.stance)}" data-testid="drawer-stance">{stanceChip(member.latest.stance, member.latest.confidence)}</p>
					<p class="num text-2xs text-faint">{member.latest.symbol} · {clock(member.latest.at)}</p>
					<ul class="mt-1 list-disc pl-4 text-mute">
						{#each member.latest.evidence as e, i (i)}<li>{e}</li>{/each}
					</ul>
				{:else}
					<p class="text-mute">No opinion yet. Click Scan now to run the members.</p>
				{/if}
			</section>
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">How the stance is made</h3>
				<p class="text-mute" data-testid="drawer-how">{rules.how}</p>
				<p class="mt-1 text-faint">{COMMON}</p>
			</section>
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">Weight &amp; inputs</h3>
				<dl class="num grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
					<dt class="text-faint">Weight (live{map.live.version !== null ? ` v${map.live.version}` : ''})</dt>
					<dd>{member.weight.toFixed(2)}</dd>
					<dt class="text-faint">Default</dt>
					<dd>{map.defaults.weights[member.agent].toFixed(2)}</dd>
					<dt class="text-faint">Opinions</dt>
					<dd>{member.latest?.opinions ?? 0}</dd>
				</dl>
				<p class="mt-1.5 text-mute">{rules.inputs}</p>
			</section>
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">Recent opinions</h3>
				{#if recent.length === 0}
					<p class="text-mute">None loaded.</p>
				{:else}
					<table class="w-full">
						<tbody>
							{#each recent as r (r.d.id)}
								<tr class="border-t border-line/60 first:border-0">
									<td class="num py-1 text-mute">{clock(r.d.created_ms)}</td>
									<td class="py-1">{r.d.symbol}</td>
									<td class="num py-1 {stanceClass(r.o.stance)}">{stanceChip(r.o.stance, r.o.confidence)}</td>
									<td class="num py-1 text-right text-faint">{r.o.outcome ?? signed(r.o.contribution, 3)}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				{/if}
			</section>
		{:else if nodeId === 'friday'}
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">Role</h3>
				<p class="text-mute">Reads the council's packet and the members' tool results, then proposes one plan or stands aside. Read and propose tools only. Pilot and your confirm execute.</p>
			</section>
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">Model &amp; cost</h3>
				<dl class="num grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
					<dt class="text-faint">Provider</dt><dd>{map.master.provider}</dd>
					<dt class="text-faint">Model</dt><dd>{map.master.model ?? DASH}</dd>
					<dt class="text-faint">This month</dt>
					<dd>{map.master.spend ? `$${map.master.spend.spent_usd.toFixed(2)} of $${map.master.spend.budget_usd.toFixed(0)} Desk cap` : DASH}</dd>
					<dt class="text-faint">Auto-ask</dt><dd>off</dd>
				</dl>
			</section>
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">Prompt</h3>
				<p class="text-mute">Each decision keeps its system prompt hash, the exact messages and tool results sent, the raw reply and the parsed plan. Open a decision and expand "Exact prompt &amp; inputs".</p>
			</section>
		{:else if nodeId === 'sentinel'}
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">Guards (Desk{map.live.version !== null ? ` · v${map.live.version}` : ''})</h3>
				<dl class="num grid grid-cols-[1fr_auto] gap-x-3 gap-y-1" data-testid="drawer-guards">
					{#each GUARD_LABEL as [k, label] (k)}
						<dt class="text-mute">{label}</dt><dd>{guards[k]}</dd>
					{/each}
				</dl>
				<p class="mt-2 text-faint">Sentinel is the council's risk seat and can veto. Its rules also check every plan Friday proposes. Changing them needs Settings, not this page.</p>
			</section>
		{:else if info}
			<section class="border border-line bg-deck p-3">
				<h3 class="hud-title mb-1.5">What it does</h3>
				{#each info.body as b, i (i)}<p class="text-mute {i > 0 ? 'mt-1' : ''}">{b}</p>{/each}
				{#if nodeId === 'chair'}
					<p class="num mt-2 text-ink">threshold {guards.min_council_score.toFixed(2)} · at least {guards.min_non_neutral} non-neutral · live weights {map.live.version !== null ? `v${map.live.version}` : 'default'}</p>
				{/if}
			</section>
		{/if}
		{#if latest && !member && nodeId !== 'friday'}
			<p class="text-2xs text-faint">Latest decision: {latest.symbol} {clock(latest.created_ms)}</p>
		{/if}
	</div>
</div>
