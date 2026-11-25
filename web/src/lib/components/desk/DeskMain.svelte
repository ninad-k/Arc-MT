<script lang="ts">
	import type { Snippet } from 'svelte';
	import Panel from '#lib/components/Panel.svelte';
	import Segmented from '#lib/components/Segmented.svelte';
	import DeskRing from './DeskRing.svelte';
	import MarketPulse from './MarketPulse.svelte';
	import SignalLog from './SignalLog.svelte';
	import SessionClock from './SessionClock.svelte';
	import DeskCalendar from './DeskCalendar.svelte';
	import AgentLoad from './AgentLoad.svelte';
	import DeskChart from './DeskChart.svelte';
	import PlanCards from './PlanCards.svelte';
	import Findings from './Findings.svelte';
	import Journal from './Journal.svelte';
	import { desk } from '#lib/state/desk.svelte.ts';

	let { chat }: { chat: Snippet } = $props();

	$effect(() => desk.start());

	type Tab = 'desk' | 'chart' | 'intel' | 'plans' | 'clock';
	let tab = $state<Tab>('desk');
	const tabs = $derived([
		{ id: 'desk' as const, label: 'Desk' },
		{ id: 'chart' as const, label: 'Chart' },
		{ id: 'intel' as const, label: 'Signals' },
		{ id: 'plans' as const, label: 'Plans', badge: desk.plans.length ? String(desk.plans.length) : undefined },
		{ id: 'clock' as const, label: 'Clock' }
	]);
	const show = (t: Tab) => (tab === t ? '' : 'max-lg:hidden');

	type Side = 'plans' | 'findings' | 'journal';
	let side = $state<Side>('plans');
	const sideTabs = $derived([
		{ id: 'plans' as const, label: 'Plans', badge: desk.plans.length ? String(desk.plans.length) : undefined },
		{ id: 'findings' as const, label: 'Findings' },
		{ id: 'journal' as const, label: 'Journal' }
	]);
</script>

<main
	class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4 lg:grid lg:overflow-visible pb-[max(1rem,env(safe-area-inset-bottom))] lg:grid-cols-[17rem_minmax(0,1fr)_19rem] lg:grid-rows-[minmax(16rem,1fr)_minmax(18rem,1.1fr)] lg:gap-4 xl:grid-cols-[18rem_minmax(0,1fr)_21rem]"
	data-testid="desk-main"
>
	<Segmented class="lg:hidden" label="Desk panels" options={tabs} bind:value={tab} />

	{#if desk.error}
		<p class="border border-down/50 bg-down/10 px-3 py-1.5 text-xs text-down lg:col-span-3" role="alert" data-testid="desk-error">
			{desk.error}
		</p>
	{/if}
	{#if desk.state && !desk.state.enabled}
		<p class="border border-warn/50 bg-warn/10 px-3 py-1.5 text-xs text-warn lg:col-span-3" role="status">
			{desk.state.reason ?? 'Desk mode is turned off on the server.'}
		</p>
	{:else if desk.state?.brain && !desk.state.brain.available}
		<p class="border border-warn/50 bg-warn/10 px-3 py-1.5 text-xs text-warn lg:col-span-3" role="status" data-testid="desk-brain-missing">
			Friday's model is not available ({desk.state.brain.reason}). The panels still work; questions need the model.
		</p>
	{/if}

	<!-- Left: market pulse, task queue, signal log. -->
	<div class="flex min-h-0 flex-col gap-3 max-lg:flex-1 lg:col-start-1 lg:row-span-2 lg:row-start-1 lg:gap-4 {show('intel')}">
		<MarketPulse />
		<SignalLog />
	</div>

	<!-- Centre top: the ring of seats around Friday, and the chart. -->
	<div class="grid shrink-0 gap-3 lg:min-h-0 lg:col-start-2 lg:row-start-1 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)] lg:gap-4 {tab === 'desk' || tab === 'chart' ? '' : 'max-lg:hidden'}">
		<div class="h-[clamp(16rem,42dvh,24rem)] min-h-0 lg:h-auto {show('desk')}"><DeskRing /></div>
		<div class="h-[clamp(14rem,40dvh,22rem)] min-h-0 lg:h-auto {show('chart')}"><DeskChart /></div>
	</div>

	<!-- Centre bottom: the conversation with Friday. -->
	<div class="min-h-[22rem] flex-1 lg:min-h-0 lg:col-start-2 lg:row-start-2 {show('desk')}">
		{@render chat()}
	</div>

	<!-- Right: clock, calendar, plans / findings / journal, load. -->
	<div class="flex min-h-0 flex-col gap-3 overflow-y-auto lg:col-start-3 lg:row-span-2 lg:row-start-1 lg:gap-4 {tab === 'clock' || tab === 'plans' ? '' : 'max-lg:hidden'}">
		<div class={show('clock')}><SessionClock /></div>
		<div class={show('clock')}><DeskCalendar /></div>
		<div class="min-h-56 {show('plans')}">
			<Panel title="The book" class="h-full">
				{#snippet meta()}
					<Segmented label="Book" options={sideTabs} bind:value={side} />
				{/snippet}
				{#if side === 'plans'}
					<PlanCards />
				{:else if side === 'findings'}
					<Findings />
				{:else}
					<Journal />
				{/if}
			</Panel>
		</div>
		<div class={show('clock')}><AgentLoad /></div>
	</div>
</main>
