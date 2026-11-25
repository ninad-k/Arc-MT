<script lang="ts">
	import type { ChatMode } from '#lib/api.ts';
	import { mode } from '#lib/state/mode.svelte.ts';
	import { assistant } from '#lib/state/assistant.svelte.ts';
	import { preview } from '#lib/state/preview.svelte.ts';
	import { chat } from '#lib/state/chat.svelte.ts';
	import { desk } from '#lib/state/desk.svelte.ts';

	let { class: klass = '' }: { class?: string } = $props();

	const options: { id: ChatMode; label: string; title: string }[] = [
		{ id: 'trading', label: 'Trading', title: 'Jarvis or Friday on MetaTrader 5' },
		{ id: 'assistant', label: 'Assistant', title: 'Jarvis on Claude: 3D models, your notes, "remember that". No trading.' },
		{ id: 'desk', label: 'Desk', title: 'Nimbus Desk: Friday and eight specialists. Trades only through Sentinel and your confirmation.' }
	];

	/** An open trade confirmation keeps the mode where it is. A reply in flight does not: it
	 * lands in the conversation it was asked in. */
	const lock = $derived(preview.isOpen ? 'Finish or cancel the open confirmation first' : null);

	function choose(next: ChatMode) {
		if (lock || next === mode.value) return;
		mode.set(next);
		if (next === 'assistant') {
			void assistant.loadStatus();
			void assistant.loadRecent();
		}
		if (next === 'desk') void desk.load();
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
		e.preventDefault();
		const ids = options.map((o) => o.id);
		const i = ids.indexOf(mode.value);
		const step = e.key === 'ArrowRight' ? 1 : ids.length - 1;
		choose(ids[(i + step) % ids.length]);
		const group = e.currentTarget as HTMLElement;
		group.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')?.focus();
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
	role="group"
	aria-label="Mode"
	class="flex h-7 border border-line bg-panel/70 p-0.5 {lock ? 'opacity-60' : ''} {klass}"
	title={lock ?? undefined}
	data-testid="mode-switch"
	{onkeydown}
>
	{#each options as o (o.id)}
		{@const on = o.id === mode.value}
		<button
			type="button"
			data-mode={o.id}
			aria-pressed={on}
			disabled={lock !== null && !on}
			title={o.title}
			class="px-2.5 text-2xs font-medium tracking-wider uppercase transition-colors disabled:cursor-not-allowed {on
				? 'bg-accent/15 text-accent'
				: 'text-mute hover:text-ink'}"
			onclick={() => choose(o.id)}
		>
			{o.label}
		</button>
	{/each}
</div>
