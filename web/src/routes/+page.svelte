<script lang="ts">
	import { onMount } from 'svelte';
	import TopBar from '#lib/components/TopBar.svelte';
	import Orb from '#lib/components/Orb.svelte';
	import Watchlist from '#lib/components/Watchlist.svelte';
	import AlertsPanel from '#lib/components/AlertsPanel.svelte';
	import AlertDialog from '#lib/components/AlertDialog.svelte';
	import AccountStrip from '#lib/components/AccountStrip.svelte';
	import Positions from '#lib/components/Positions.svelte';
	import MarketView from '#lib/components/MarketView.svelte';
	import Panel from '#lib/components/Panel.svelte';
	import Transcript from '#lib/components/Transcript.svelte';
	import Composer from '#lib/components/Composer.svelte';
	import Segmented from '#lib/components/Segmented.svelte';
	import ConfirmModal from '#lib/components/ConfirmModal.svelte';
	import { positions, startPolling, watchlist } from '#lib/state/hud.svelte.ts';
	import { chat } from '#lib/state/chat.svelte.ts';
	import { auth } from '#lib/state/auth.svelte.ts';
	import { events } from '#lib/state/events.svelte.ts';
	import { killSwitch } from '#lib/state/killswitch.svelte.ts';
	import { preview } from '#lib/state/preview.svelte.ts';
	import { attachPushToTalk } from '#lib/state/voice.svelte.ts';
	import { persona } from '#lib/state/persona.svelte.ts';
	import { alerts } from '#lib/state/alerts.svelte.ts';
	import { briefing } from '#lib/state/briefing.svelte.ts';
	import { market } from '#lib/state/market.svelte.ts';
	import { mode } from '#lib/state/mode.svelte.ts';
	import { assistant } from '#lib/state/assistant.svelte.ts';
	import ModelStage from '#lib/components/ModelStage.svelte';
	import DeskMain from '#lib/components/desk/DeskMain.svelte';
	import ImageUploadSheet from '#lib/components/ImageUploadSheet.svelte';
	import { uploads } from '#lib/state/uploads.svelte.ts';

	// Dropping a file (spec-model-uploads): a cover over the whole window takes the drop, so it
	// works over a model too (the 3D frame would swallow it), and a file dropped elsewhere never
	// makes the browser leave the app.
	let dropping = $state(false);
	let dragDepth = 0;
	const hasFiles = (e: DragEvent) => e.dataTransfer?.types.includes('Files') === true;
	function onDragEnter(e: DragEvent) {
		if (!mode.assistant || !hasFiles(e)) return;
		dragDepth++;
		dropping = true;
	}
	function onDragLeave(e: DragEvent) {
		if (!dropping || !hasFiles(e)) return;
		dragDepth = Math.max(0, dragDepth - 1);
		if (dragDepth === 0) dropping = false;
	}
	function onWindowDrop(e: DragEvent) {
		if (hasFiles(e)) e.preventDefault();
		dragDepth = 0;
		dropping = false;
	}
	function onDrop(e: DragEvent) {
		e.preventDefault();
		e.stopPropagation();
		dragDepth = 0;
		dropping = false;
		const file = e.dataTransfer?.files?.[0];
		if (file) void uploads.open(file);
	}

	type Tab = 'chat' | 'watchlist' | 'positions' | 'market';
	let tab = $state<Tab>('chat');

	onMount(() => {
		const stops = [startPolling(), events.start(), attachPushToTalk(), alerts.start()];
		void auth.refresh();
		void persona.ensureLoaded();
		return () => stops.forEach((stop) => stop());
	});

	// Engaging the kill switch revokes every live token: close the open confirmation with a reason.
	$effect(() => {
		if (killSwitch.engaged) preview.onKillSwitch();
	});

	const tabs = $derived([
		{ id: 'chat' as const, label: 'Chat' },
		{ id: 'watchlist' as const, label: 'Watchlist', badge: watchlist.data ? String(watchlist.data.prices.length) : undefined },
		{ id: 'positions' as const, label: 'Positions', badge: positions.data ? String(positions.data.length) : undefined },
		{ id: 'market' as const, label: 'Market' }
	]);

	// From lg up the three columns are always visible; the middle one shows either the chat (under
	// the orb) or the market view, which takes the whole column because it is tall.
	const center = $derived(tab === 'market' ? 'market' : 'chat');

	// Below lg only the selected tab's panel is shown; at lg everything is visible.
	const show = (t: Tab) => (tab === t ? '' : 'max-lg:hidden');

	// Assistant mode: know whether Claude is usable and what it has built.
	$effect(() => {
		if (!mode.assistant) return;
		void assistant.loadStatus();
		void assistant.loadRecent();
	});

	/** "Brief me" from the chat header: open the Market panel on the briefing and build it. */
	function briefMe() {
		tab = 'market';
		market.setTab('briefing');
		if (!briefing.loading) void briefing.load();
	}
</script>

<svelte:window
	ondragenter={onDragEnter}
	ondragleave={onDragLeave}
	ondragover={(e) => {
		if (hasFiles(e)) e.preventDefault();
	}}
	ondrop={onWindowDrop}
/>

<ConfirmModal />
<ImageUploadSheet />
{#if dropping}
	<div
		class="fixed inset-0 z-50 grid place-items-center border-4 border-dashed border-accent bg-void/70 text-base text-ink backdrop-blur-sm"
		role="region"
		aria-label="Drop a 3D file or a picture to open it"
		ondragover={(e) => e.preventDefault()}
		ondrop={onDrop}
		data-testid="drop-overlay"
	>
		Drop to open in 3D
	</div>
{/if}
<AlertDialog />

{#snippet chatPanel()}
	<Panel title="Transcript" class="h-full" scroll={false}>
		{#snippet meta()}
			{#if chat.sessionId}
				<span title="Session {chat.sessionId}">session {chat.sessionId.slice(0, 8)}</span>
			{/if}
			{#if !mode.assistant && !mode.desk}
				<button
					type="button"
					class="border border-line px-2 py-0.5 text-mute transition-colors hover:border-accent hover:text-ink max-lg:hidden"
					onclick={() => (tab = 'market')}
				>
					Market view
				</button>
				<button
					type="button"
					class="border border-line px-2 py-0.5 text-mute transition-colors hover:border-accent hover:text-ink"
					title="A summary of your account, positions, today's events and the watchlist"
					onclick={briefMe}
				>
					Brief me
				</button>
			{/if}
			{#if chat.messages.length > 0}
				<button
					type="button"
					class="border border-line px-2 py-0.5 text-mute transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
					disabled={chat.pending}
					onclick={() => chat.reset()}
				>
					New chat
				</button>
			{/if}
		{/snippet}
		<Transcript />
		<Composer />
	</Panel>
{/snippet}

<div class="flex h-dvh flex-col">
	<TopBar />

	{#if mode.desk}
		<!-- Desk mode (spec-nimbus-desk): Friday and the eight specialists around the orb. -->
		<DeskMain chat={chatPanel} />
	{:else if mode.assistant}
		<!-- Assistant mode: the stage (the orb, or the 3D model on screen) and the chat. No MT5 panels. -->
		<main
			class="grid min-h-0 flex-1 grid-cols-1 grid-rows-[minmax(12rem,1.1fr)_minmax(0,1fr)] gap-3 p-4 pb-[max(1rem,env(safe-area-inset-bottom))] lg:grid-cols-[minmax(0,1.5fr)_minmax(22rem,1fr)] lg:grid-rows-1 lg:gap-4"
			data-testid="assistant-main"
		>
			<div class="relative min-h-0" data-testid="stage">
				{#if assistant.stageModel}
					<ModelStage model={assistant.stageModel} />
				{:else}
					<Orb />
				{/if}
			</div>
			<div class="flex min-h-0 flex-col gap-2">
				{#if uploads.busy}
					<p class="border border-accent/40 bg-accent/10 px-3 py-1.5 text-xs text-ink" role="status" data-testid="upload-busy">
						{uploads.busy}
					</p>
				{/if}
				{#if assistant.notice}
					<p
						class="border px-3 py-1.5 text-xs {assistant.notice.tone === 'bad'
							? 'border-down/50 bg-down/10 text-down'
							: 'border-warn/50 bg-warn/10 text-warn'}"
						role="status"
						data-testid="assistant-notice"
					>
						{assistant.notice.text}
					</p>
				{/if}
				<div class="min-h-0 flex-1">{@render chatPanel()}</div>
			</div>
		</main>
	{:else}
	<main
		class="grid min-h-0 flex-1 grid-cols-1 grid-rows-[auto_auto_minmax(0,1fr)] gap-3 p-4 pb-[max(1rem,env(safe-area-inset-bottom))] lg:grid-cols-[18rem_minmax(0,1fr)_19rem] xl:grid-cols-[19rem_minmax(0,1fr)_24rem] lg:grid-rows-[minmax(14rem,1fr)_minmax(18rem,1.15fr)] lg:gap-4"
	>
		<!-- Orb -->
		<div class="h-[clamp(9rem,24dvh,15rem)] min-h-0 md:h-[clamp(12rem,30dvh,22rem)] lg:col-start-2 lg:row-start-1 lg:h-auto {tab === 'market' ? 'max-md:h-[6rem]' : ''} {center === 'market' ? 'lg:hidden' : ''}">
			<Orb />
		</div>

		<Segmented class="lg:hidden" label="Panels" options={tabs} bind:value={tab} />

		<!-- Watchlist, with the alerts under it: alerts are made from a watchlist row, so they live together. -->
		<div
			id="panel-watchlist"
			role="tabpanel"
			aria-labelledby="tab-watchlist"
			class="flex min-h-0 flex-col gap-3 lg:col-start-1 lg:row-span-2 lg:row-start-1 lg:gap-4 {show('watchlist')}"
		>
			<div class="min-h-24 flex-1 lg:min-h-48"><Watchlist /></div>
			<AlertsPanel />
		</div>

		<!-- Chat -->
		<div
			id="panel-chat"
			role="tabpanel"
			aria-labelledby="tab-chat"
			class="min-h-0 lg:col-start-2 lg:row-start-2 {center === 'market' ? 'lg:hidden' : ''} {show('chat')}"
		>
			{@render chatPanel()}
		</div>

		<!-- Market view -->
		<div
			id="panel-market"
			role="tabpanel"
			aria-labelledby="tab-market"
			class="min-h-0 lg:col-start-2 lg:row-span-2 lg:row-start-1 {center === 'market' ? '' : 'lg:hidden'} {show('market')}"
		>
			<MarketView onclose={() => (tab = 'chat')} />
		</div>

		<!-- Account + positions -->
		<div
			id="panel-positions"
			role="tabpanel"
			aria-labelledby="tab-positions"
			class="min-h-0 flex-col gap-3 lg:col-start-3 lg:row-span-2 lg:row-start-1 lg:flex lg:gap-4 {tab === 'positions' ? 'flex' : 'max-lg:hidden'}"
		>
			<AccountStrip />
			<div class="min-h-0 flex-1"><Positions /></div>
		</div>
	</main>
	{/if}
</div>
