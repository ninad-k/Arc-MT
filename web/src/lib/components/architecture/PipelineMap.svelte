<script lang="ts">
	import {
		executionRows,
		signed,
		stanceBg,
		stanceChip,
		stanceClass,
		tierDim,
		tierStopped
	} from '#lib/architecture/format.ts';
	import type { ArchitectureMap, DecisionDetail, Stage } from '#lib/architecture/types.ts';

	let {
		map,
		latest,
		selected,
		onselect
	}: {
		map: ArchitectureMap;
		latest: DecisionDetail | null;
		selected: string | null;
		onselect: (id: string) => void;
	} = $props();

	const c = $derived(latest?.council ?? null);
	const exec = $derived(latest ? executionRows(latest) : []);
	const execState = (id: string) => exec.find((r) => r.id === id)?.state ?? 'skip';
	const vetoed = $derived(!!c?.stop && c.stop[0].startsWith('sentinel_veto'));

	const names = (pts: { agent: string }[]) => pts.map((p) => p.agent.charAt(0).toUpperCase() + p.agent.slice(1)).join(', ') || 'none';
	const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

	const SIGNALS = [
		{ id: 'sig_scout', name: 'Scout movers', role: 'biggest movers on the watchlist' },
		{ id: 'sig_view', name: 'Market view', role: 'bias plus setup' },
		{ id: 'sig_calendar', name: 'Calendar & tone', role: 'events and headlines' },
		{ id: 'sig_question', name: 'Your question', role: 'asked in Desk' }
	];

	const tiers: { id: Stage; n: number; title: string }[] = [
		{ id: 'signal', n: 1, title: 'Signals' },
		{ id: 'members', n: 2, title: 'General members' },
		{ id: 'council', n: 3, title: 'Council' },
		{ id: 'master', n: 4, title: 'Master' },
		{ id: 'execution', n: 5, title: 'Execution' }
	];
	const tierClass = (t: Stage) =>
		tierStopped(t, latest)
			? 'border-down/60 shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--color-down)_25%,transparent)]'
			: latest?.status === 'waiting' && latest.stage === t
				? 'border-warn/60'
				: 'border-line';
	const tierNote = (t: Stage): string => {
		if (!latest) return '';
		if (tierStopped(t, latest)) return 'stopped';
		if (latest.status === 'waiting' && latest.stage === t) return 'waiting';
		if (tierDim(t, latest)) return 'not reached';
		return '';
	};
	const execDot = { ok: 'bg-up', fail: 'bg-down', skip: 'bg-faint' } as const;
	const execTxt = { ok: 'ok', fail: 'stop', skip: '—' } as const;
	const execCls = { ok: 'text-up', fail: 'text-down', skip: 'text-faint' } as const;
</script>

{#snippet node(id: string, dot: string, name: string, role: string, chip: string, chipCls: string)}
	<button
		type="button"
		class="grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 border bg-panel px-2 py-1.5 text-left transition-colors hover:border-accent focus-visible:border-accent {selected === id
			? 'border-accent shadow-[inset_0_0_0_1px_var(--color-accent)]'
			: 'border-line'}"
		aria-label="{name}: {role}. Open details"
		aria-haspopup="dialog"
		data-testid="node-{id}"
		onclick={() => onselect(id)}
	>
		<span class="size-2 rounded-full {dot}" aria-hidden="true"></span>
		<span class="min-w-0 text-xs font-semibold">
			{name}
			<span class="block truncate text-[11px] font-normal text-faint">{role}</span>
		</span>
		<span class="num text-[11px] {chipCls}">{chip}</span>
	</button>
{/snippet}

<div class="pipeline" data-testid="pipeline-map">
	{#each tiers as t (t.id)}
		{#if t.n > 1}
			<div class="arrow {tierStopped(t.id, latest) ? 'text-down' : 'text-line-hi'}" aria-hidden="true">➜</div>
		{/if}
		<section
			class="flex flex-col gap-1.5 border bg-deck p-2.5 {tierClass(t.id)} {tierDim(t.id, latest) ? 'opacity-50' : ''}"
			aria-label="{t.n}. {t.title}"
			data-testid="tier-{t.id}"
			data-stopped={tierStopped(t.id, latest)}
		>
			<div class="flex justify-between text-[10px] tracking-[0.14em] text-faint uppercase">
				<span>{t.n} · {t.title}</span>
				<span class={tierStopped(t.id, latest) ? 'text-down' : latest?.status === 'waiting' && latest.stage === t.id ? 'text-warn' : ''}>
					{#if t.id === 'members'}{map.members.length} opinions{:else}{tierNote(t.id)}{/if}
				</span>
			</div>

			{#if t.id === 'signal'}
				{#each SIGNALS as s (s.id)}
					{@render node(s.id, 'bg-violet', s.name, s.role, '', 'text-faint')}
				{/each}
				{#if latest}
					<p class="px-1 text-[11px] text-faint" data-testid="latest-trigger">Latest: {latest.trigger.detail ?? latest.trigger.kind}</p>
				{/if}
			{:else if t.id === 'members'}
				{#each map.members as m (m.agent)}
					{@render node(
						m.agent,
						m.latest ? stanceBg(m.latest.stance) : 'bg-faint',
						cap(m.agent),
						m.title,
						stanceChip(m.latest?.stance, m.latest?.confidence),
						m.latest ? stanceClass(m.latest.stance) : 'text-faint'
					)}
				{/each}
			{:else if t.id === 'council'}
				{@render node('bull_case', 'bg-up', 'Bull case', c ? names(c.bull.points) : 'strongest weighted evidence', c ? signed(c.bull.total) : '—', 'text-up')}
				{@render node('bear_case', 'bg-down', 'Bear case', c ? names(c.bear.points) : 'strongest weighted evidence', c ? signed(c.bear.total) : '—', 'text-down')}
				{@render node('sentinel', vetoed ? 'bg-down' : c ? 'bg-up' : 'bg-faint', 'Risk · Sentinel', vetoed ? 'veto' : 'no veto', c ? (vetoed ? 'veto' : 'pass') : '—', vetoed ? 'text-down' : c ? 'text-up' : 'text-faint')}
				{@render node('chair', c ? (c.passed ? 'bg-up' : 'bg-down') : 'bg-faint', 'Chair', 'score vs threshold', c ? `${signed(c.score)} ${Math.abs(c.score) >= c.threshold ? '≥' : '<'} ${c.threshold.toFixed(2)}` : '—', c ? (c.passed ? 'text-up' : 'text-down') : 'text-faint')}
			{:else if t.id === 'master'}
				{@render node('friday', 'bg-violet', 'Friday', map.master.model ?? 'plan + exact prompt', latest?.master ? 'asked' : '—', 'text-violet')}
				<p class="px-1 text-[11.5px] text-faint">Paid step. Runs on a council-passed decision when you click <b class="text-accent">Ask Friday</b>.</p>
			{:else}
				{@render node('pilot', execDot[execState('pilot')], 'Pilot', 'preview', execTxt[execState('pilot')], execCls[execState('pilot')])}
				{@render node('gate', execDot[execState('gate')], 'Gate', '[limits] · kill switch · lock', execTxt[execState('gate')], execCls[execState('gate')])}
				{@render node('confirm', execDot[execState('confirm')], 'Your confirm', '15 s token', execTxt[execState('confirm')], execCls[execState('confirm')])}
				{@render node('broker', execDot[execState('broker')], 'Broker', 'MT5 · Alpaca paper', execTxt[execState('broker')], execCls[execState('broker')])}
			{/if}
		</section>
	{/each}
</div>

<style>
	.pipeline {
		display: grid;
		grid-template-columns: 1fr 24px 1.35fr 24px 1.1fr 24px 1fr 24px 1.1fr;
		align-items: stretch;
	}
	.arrow {
		display: flex;
		align-items: center;
		justify-content: center;
		font-size: 18px;
	}
	@media (max-width: 1100px) {
		.pipeline {
			grid-template-columns: minmax(0, 1fr);
			gap: 6px;
		}
		.arrow {
			transform: rotate(90deg);
			height: 18px;
		}
	}
</style>
