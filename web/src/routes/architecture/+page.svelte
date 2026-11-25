<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ConfirmModal from '#lib/components/ConfirmModal.svelte';
	import ConsensusBar from '#lib/components/architecture/ConsensusBar.svelte';
	import ContributionBars from '#lib/components/architecture/ContributionBars.svelte';
	import DecisionDetail from '#lib/components/architecture/DecisionDetail.svelte';
	import DecisionTable from '#lib/components/architecture/DecisionTable.svelte';
	import LearningTab from '#lib/components/architecture/LearningTab.svelte';
	import LockChip from '#lib/components/architecture/LockChip.svelte';
	import Funnel from '#lib/components/architecture/Funnel.svelte';
	import NodeDrawer from '#lib/components/architecture/NodeDrawer.svelte';
	import PipelineMap from '#lib/components/architecture/PipelineMap.svelte';
	import StopBanner from '#lib/components/architecture/StopBanner.svelte';
	import { relativeTime } from '#lib/architecture/format.ts';
	import { countdown } from '#lib/desk/format.ts';
	import { architecture } from '#lib/state/architecture.svelte.ts';
	import { killSwitch } from '#lib/state/killswitch.svelte.ts';
	import { preview } from '#lib/state/preview.svelte.ts';

	type Tab = 'pipeline' | 'decisions' | 'learning';
	const TABS: { id: Tab; label: string }[] = [
		{ id: 'pipeline', label: 'Pipeline' },
		{ id: 'decisions', label: 'Decisions' },
		{ id: 'learning', label: 'Learning' }
	];

	const tab = $derived.by<Tab>(() => {
		const t = page.url.searchParams.get('tab');
		return t === 'decisions' || t === 'learning' ? t : 'pipeline';
	});
	const decisionId = $derived(page.url.searchParams.get('d'));

	let node = $state<string | null>(null);
	let detailEl = $state<HTMLElement>();
	let now = $state(Date.now());

	function go(t: Tab, d: string | null = null) {
		const q = new URLSearchParams({ tab: t });
		if (d) q.set('d', d);
		void goto(`?${q}`, { reset: false });
	}

	onMount(() => {
		void architecture.loadPipeline();
		const t = setInterval(() => {
			now = Date.now();
		}, 30_000);
		const r = setInterval(() => void architecture.loadMap(), 30_000);
		return () => {
			clearInterval(t);
			clearInterval(r);
		};
	});

	// Each tab loads what it shows when it opens.
	$effect(() => {
		if (tab === 'decisions') void architecture.loadDecisions();
	});
	// A decision in the URL is the open decision.
	$effect(() => {
		const id = decisionId;
		void architecture.open(id);
		if (id)
			void tick().then(() => detailEl?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' }));
	});
	// Engaging the kill switch revokes every live token: close the open confirmation with a reason.
	$effect(() => {
		if (killSwitch.engaged) preview.onKillSwitch();
	});

	async function scan() {
		const before = decisionId;
		const id = await architecture.scan();
		// Open the new decision only if the trader did not pick another one while it ran.
		if (id && tab === 'decisions' && decisionId === before) go('decisions', id);
	}

	const map = $derived(architecture.map);
	const latest = $derived(architecture.latest);
	const session = $derived(map?.session ?? null);
	const sessionText = $derived.by(() => {
		if (!session) return 'NY session —';
		if (session.phase === 'open') return `NY session · ${countdown(session.seconds_to_close)} to close`;
		return `NY ${session.label.toLowerCase()}${session.seconds_to_change !== null ? ` · ${countdown(session.seconds_to_change)} to ${session.next_phase?.replace('_', '-') ?? 'change'}` : ''}`;
	});
	const sessionDot = $derived(session?.phase === 'open' ? 'bg-up' : session?.phase === 'closed' ? 'bg-faint' : 'bg-warn');
</script>

<svelte:head><title>Architecture · Arc MT</title></svelte:head>

<ConfirmModal />

<div class="flex h-dvh flex-col">
	<header
		class="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-line bg-deck/80 px-4 pt-[max(0.5rem,env(safe-area-inset-top))] pb-2 backdrop-blur"
	>
		<a href="/" class="num text-xs text-mute hover:text-ink">← HUD</a>
		<h1 class="font-mono text-sm font-semibold tracking-[0.2em] uppercase">Architecture</h1>

		<div role="tablist" aria-label="Architecture views" class="flex gap-0.5 border border-line bg-panel p-0.5">
			{#each TABS as t (t.id)}
				<button
					type="button"
					role="tab"
					id="tab-{t.id}"
					aria-selected={tab === t.id}
					aria-current={tab === t.id ? 'page' : undefined}
					aria-controls="panel-{t.id}"
					class="px-3 py-1 text-xs transition-colors {tab === t.id ? 'bg-raise text-ink shadow-[inset_0_0_0_1px_var(--color-line-hi)]' : 'text-mute hover:text-ink'}"
					data-testid="tab-{t.id}"
					onclick={() => go(t.id, t.id === 'decisions' ? decisionId : null)}
				>
					{t.label}
				</button>
			{/each}
		</div>

		<div class="ml-auto flex flex-wrap items-center gap-2">
			<!-- LOCK-CHIP-SLOT: the master account lock chip (opens its drawer). -->
			<LockChip />
			<span class="pill border-line-hi text-mute max-sm:hidden" data-testid="session-line">
				<span class="size-2 rounded-full {sessionDot}" aria-hidden="true"></span>
				{sessionText}
			</span>
			{#if map}
				<span class="pill border-line-hi text-mute max-md:hidden normal-case tracking-normal" data-testid="scan-line">
					{latest ? `last scan ${relativeTime(latest.created_ms, now)} · ` : ''}every {map.scan_every_minutes} min
				</span>
			{/if}
			<button type="button" class="btn btn-primary" disabled={architecture.scanning} onclick={scan} data-testid="scan-now">
				{architecture.scanning ? 'Scanning…' : 'Scan now'}
				<span class="font-normal text-faint normal-case tracking-normal">· free</span>
			</button>
		</div>
	</header>

	<main class="min-h-0 flex-1 overflow-x-hidden overflow-y-auto p-3 sm:p-4">
		<div class="mx-auto grid max-w-[1440px] gap-3">
			{#if architecture.scanError}
				<p class="border border-down/60 bg-down/10 px-3 py-2 text-xs text-down" role="alert">{architecture.scanError}</p>
			{:else if architecture.scanNote}
				<p class="border border-up/40 bg-up/5 px-3 py-2 text-xs text-up" role="status" data-testid="scan-note">{architecture.scanNote}</p>
			{/if}
			{#if architecture.mapError}
				<div class="flex items-center justify-between gap-3 border border-down/60 bg-down/10 px-3 py-2 text-xs text-down" role="alert">
					<span>{architecture.mapError}</span>
					<button type="button" class="underline" onclick={() => void architecture.loadPipeline()}>Retry</button>
				</div>
			{/if}

			<!-- ============ PIPELINE ============ -->
			<div id="panel-pipeline" role="tabpanel" aria-labelledby="tab-pipeline" hidden={tab !== 'pipeline'} class="grid gap-3">
				{#if tab === 'pipeline'}
					<div class="hud p-3">
						<h2 class="hud-title mb-2 flex flex-wrap items-center gap-2">
							Decision pipeline
							{#if latest}<span class="text-faint normal-case tracking-normal">· latest: {latest.symbol}</span>{/if}
							{#if map}<span class="pill ml-auto border-line-hi text-mute">weights {map.live.version !== null ? `v${map.live.version}` : 'default'}</span>{/if}
						</h2>
						{#if map}
							<PipelineMap {map} {latest} selected={node} onselect={(id) => (node = id)} />
						{:else if !architecture.mapError}
							<div class="skeleton h-64" aria-busy="true" aria-label="Loading the pipeline"></div>
						{/if}
					</div>

					{#if latest}
						<StopBanner decision={latest}>
							<button type="button" class="btn h-8" onclick={() => go('decisions', latest.id)} data-testid="open-latest">Open decision ›</button>
						</StopBanner>
						{#if latest.council}
							<div class="grid gap-3 lg:grid-cols-2">
								<div class="hud p-3">
									<h2 class="hud-title mb-2">Consensus · weighted by confidence</h2>
									<ConsensusBar items={latest.opinions} />
								</div>
								<div class="hud p-3">
									<h2 class="hud-title mb-2">Contribution to council score</h2>
									<ContributionBars contributions={latest.council.contributions} score={latest.council.score} threshold={latest.council.threshold} />
								</div>
							</div>
						{/if}
					{:else if map}
						<p class="border border-line bg-panel px-3 py-4 text-center text-xs text-mute" role="status">
							No decisions yet. Click Scan now to run the members and the council over the watchlist. It is free and uses no LLM.
						</p>
					{/if}
				{/if}
			</div>

			<!-- ============ DECISIONS ============ -->
			<div id="panel-decisions" role="tabpanel" aria-labelledby="tab-decisions" hidden={tab !== 'decisions'} class="grid gap-3">
				{#if tab === 'decisions'}
					<div class="hud p-3">
						<h2 class="hud-title mb-2">Funnel · click a stage to filter</h2>
						{#if architecture.funnelError}
							<p class="mb-2 text-xs text-down" role="alert">{architecture.funnelError}</p>
						{/if}
						<Funnel
							funnel={architecture.funnel}
							active={architecture.minStep}
							onstep={(s) => (architecture.minStep = s)}
							onstop={(code) => {
								architecture.filter.stop = code;
								void architecture.loadDecisions();
							}}
						/>
					</div>
					<div class="hud p-3">
						<DecisionTable selectedId={decisionId} onselect={(id) => go('decisions', id)} />
					</div>
					<div bind:this={detailEl} class="hud p-3" aria-live="polite">
						{#if architecture.detail}
							<DecisionDetail decision={architecture.detail} onmember={(id) => (node = id)} />
						{:else if architecture.detailError}
							<p class="text-xs text-down" role="alert">{architecture.detailError}</p>
						{:else if decisionId && architecture.detailLoading}
							<div class="skeleton h-40" aria-busy="true" aria-label="Loading the decision"></div>
						{:else}
							<p class="text-xs text-mute">Select a decision to see it from signals to execution.</p>
						{/if}
					</div>
				{/if}
			</div>

			<!-- ============ LEARNING ============ -->
			<div id="panel-learning" role="tabpanel" aria-labelledby="tab-learning" hidden={tab !== 'learning'} class="grid gap-3">
				{#if tab === 'learning'}
					<LearningTab onmember={(id) => (node = id)} />
				{/if}
			</div>
		</div>
	</main>
</div>

{#if node && map}
	<NodeDrawer nodeId={node} {map} {latest} onclose={() => (node = null)} />
{/if}
