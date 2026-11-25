<script lang="ts">
	import { onMount } from 'svelte';
	import { lockChipText, lockTone } from '#lib/architecture/format.ts';
	import { learning } from '#lib/state/learning.svelte.ts';
	import LockControl from './LockControl.svelte';

	let open = $state(false);
	let chip = $state<HTMLButtonElement>();
	let panel = $state<HTMLElement>();
	let closeBtn = $state<HTMLButtonElement>();
	const uid = $props.id();

	onMount(() => void learning.loadLock());

	const v = $derived(learning.lock);
	const cls = {
		up: 'border-up/50 text-up',
		down: 'border-down/50 text-down',
		warn: 'border-warn/50 text-warn',
		mute: 'border-line-hi text-mute'
	} as const;

	function close() {
		open = false;
		queueMicrotask(() => chip?.focus());
	}
	$effect(() => {
		if (open) closeBtn?.focus();
	});
	function onkeydown(e: KeyboardEvent) {
		if (!open) return;
		if (e.key === 'Escape') {
			e.stopPropagation();
			close();
			return;
		}
		if (e.key !== 'Tab' || !panel) return;
		const f = [...panel.querySelectorAll<HTMLElement>('button:not([disabled]),input,select,textarea,a[href]')].filter(
			(x) => x.offsetParent !== null
		);
		if (f.length === 0) return;
		if (e.shiftKey && document.activeElement === f[0]) {
			e.preventDefault();
			f[f.length - 1].focus();
		} else if (!e.shiftKey && document.activeElement === f[f.length - 1]) {
			e.preventDefault();
			f[0].focus();
		}
	}
</script>

<svelte:window {onkeydown} />

{#if v}
	<button
		bind:this={chip}
		type="button"
		class="pill bg-panel/60 tracking-normal normal-case hover:bg-raise {cls[lockTone(v.state)]}"
		title={v.message}
		aria-haspopup="dialog"
		onclick={() => (open = true)}
		data-testid="lock-chip"
	>
		<span aria-hidden="true">🔒</span>
		{lockChipText(v)}
	</button>
{/if}

{#if open}
	<div class="fixed inset-0 z-40 bg-void/60" aria-hidden="true" onclick={close}></div>
	<div
		bind:this={panel}
		class="fixed top-0 right-0 bottom-0 z-50 flex w-[min(440px,100%)] flex-col border-l border-line-hi bg-panel"
		role="dialog"
		aria-modal="true"
		aria-labelledby="{uid}-t"
		data-testid="lock-drawer"
	>
		<div class="flex items-center gap-3 border-b border-line px-4 py-3">
			<div>
				<h2 id="{uid}-t" class="font-semibold">Master account lock</h2>
				<p class="text-[11.5px] text-faint">MT5 · Desk master bot</p>
			</div>
			<button bind:this={closeBtn} type="button" class="btn ml-auto h-8" aria-label="Close" onclick={close} data-testid="lock-close">✕</button>
		</div>
		<div class="overflow-y-auto p-4"><LockControl /></div>
	</div>
{/if}
