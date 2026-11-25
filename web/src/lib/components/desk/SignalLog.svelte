<script lang="ts">
	import Panel from '#lib/components/Panel.svelte';
	import { desk } from '#lib/state/desk.svelte.ts';
	import { chat } from '#lib/state/chat.svelte.ts';
	import { countdown } from '#lib/desk/format.ts';

	const time = (ms: number) =>
		new Date(ms).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
	const levelClass = { info: 'text-mute', ok: 'text-up', warn: 'text-warn', error: 'text-down' } as const;
	const stateClass = { working: 'text-accent', done: 'text-up', error: 'text-down', idle: 'text-mute', offline: 'text-faint' } as const;
	let now = $state(Date.now());
	$effect(() => {
		const t = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(t);
	});
</script>

<Panel title="Task queue" class="shrink-0">
	{#snippet meta()}
		<span>{chat.pending ? 'Friday is routing…' : 'idle'}</span>
	{/snippet}
	{#if desk.queue.length === 0}
		<p class="p-3 text-xs text-mute">No tasks yet. Ask the desk something.</p>
	{:else}
		<ol class="max-h-40 divide-y divide-line/60 overflow-y-auto" data-testid="task-queue">
			{#each desk.queue as q (q.id)}
				<li class="flex items-center gap-2 px-3 py-1.5 text-2xs">
					<span class="w-16 shrink-0 font-semibold tracking-wider uppercase {stateClass[q.state]}">{q.agent}</span>
					<span class="min-w-0 flex-1 truncate text-mute" title={q.summary ?? ''}>{q.summary ?? q.state}</span>
					<span class="num shrink-0 text-faint">{countdown((now - q.at) / 1000)}</span>
				</li>
			{/each}
		</ol>
	{/if}
</Panel>

<Panel title="Signal log" class="min-h-0 flex-1">
	{@const signals = desk.state?.signals ?? []}
	{#if signals.length === 0}
		<p class="p-3 text-xs text-mute">The specialists' findings stream here.</p>
	{:else}
		<ol class="divide-y divide-line/60" data-testid="signal-log">
			{#each signals as s, i (`${s.at_ms}-${i}`)}
				<li class="px-3 py-1.5 text-2xs leading-snug">
					<div class="flex items-center gap-2">
						<span class="num text-faint">{time(s.at_ms)}</span>
						<span class="font-semibold tracking-wider uppercase {levelClass[s.level]}">{s.agent}</span>
					</div>
					<p class="mt-0.5 text-ink/90">{s.text}</p>
				</li>
			{/each}
		</ol>
	{/if}
</Panel>
