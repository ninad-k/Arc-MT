<script lang="ts">
	import { backend, status } from '#lib/state/hud.svelte.ts';
	import BridgeChip from './BridgeChip.svelte';
	import KillSwitch from './KillSwitch.svelte';
	import MainMenu from './MainMenu.svelte';
	import PersonaToggle from './PersonaToggle.svelte';
	import ProviderSelect from './ProviderSelect.svelte';
	import ModeSwitch from './ModeSwitch.svelte';
	import { mode as appMode } from '#lib/state/mode.svelte.ts';
	import { killSwitch } from '#lib/state/killswitch.svelte.ts';

	const mode = $derived(status.data?.mode ?? null);
</script>

{#if backend.isLive}
	<div
		class="bg-down px-4 py-1.5 text-center text-xs font-semibold tracking-[0.12em] text-void"
		role="alert"
	>
		LIVE ACCOUNT — real money
	</div>
{/if}

{#if killSwitch.engaged}
	<div
		class="bg-down px-4 py-1.5 text-center text-xs font-semibold tracking-[0.08em] text-void"
		role="alert"
		data-testid="kill-banner"
	>
		Trading disabled — kill switch engaged ({killSwitch.state.reason ?? 'no reason given'})
	</div>
{/if}

<header
	class="relative z-20 grid shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-2 border-b border-line bg-deck/80 px-4 pt-[max(0.5rem,env(safe-area-inset-top))] pb-2 backdrop-blur lg:flex lg:gap-x-4"
>
	<!-- One row needs about 1000 px (more once the bridge reports the account type), so below
	     lg the bar stacks into the two-row layout instead of pushing the page sideways.
	     shrink-0: the wordmark and the DEMO/LIVE badge must never be squeezed under the
	     controls next to them (the badge is a safety signal). -->
	<div class="flex min-w-0 items-center gap-3 lg:mr-auto lg:shrink-0">
		<h1 class="flex items-center gap-2 font-mono text-sm font-semibold tracking-[0.28em] whitespace-nowrap">
			<span class="grid size-5 place-items-center rounded-full border border-accent/70" aria-hidden="true">
				<span class="size-2 rounded-full bg-accent shadow-[0_0_8px_2px_color-mix(in_srgb,var(--color-accent)_70%,transparent)]"></span>
			</span>
			ARC<span class="text-accent"> MT</span>
		</h1>
		{#if mode}
			<span
				class="pill {mode === 'demo' ? 'text-up' : 'bg-down/15 text-down'}"
				title={mode === 'demo' ? 'Demo account: no real money' : 'Live trading mode'}
			>
				<span class="size-1.5 rounded-full bg-current" aria-hidden="true"></span>
				{mode === 'demo' ? 'Demo' : 'Live'}
			</span>
		{/if}
	</div>

	<div class="flex min-w-0 items-center gap-2">
		<ModeSwitch />
		<!-- Assistant mode is always Jarvis, so the persona choice belongs to Trading mode. -->
		{#if !appMode.assistant}
			<PersonaToggle />
		{/if}
	</div>

	<div class="flex items-center gap-2 max-lg:order-3">
		<BridgeChip />
		<KillSwitch />
		<a
			href="/settings"
			data-testid="settings-link"
			aria-label="Settings"
			title="Settings"
			class="grid size-7 place-items-center border border-line bg-panel/70 text-mute transition-colors hover:border-accent hover:text-ink"
		>
			<svg viewBox="0 0 24 24" class="size-3.5" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
				<circle cx="12" cy="12" r="3" />
				<path
					d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z"
					stroke-linejoin="round"
				/>
			</svg>
		</a>
		<MainMenu />
	</div>
	<ProviderSelect class="max-lg:order-2" />
</header>
