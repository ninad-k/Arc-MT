<script lang="ts">
	import { architecture } from '#lib/state/architecture.svelte.ts';
	import {
		STAGE_LABEL,
		clock,
		describeMessage,
		executionRows,
		rText,
		signed,
		stanceBg,
		stanceChip,
		stanceClass,
		usageText
	} from '#lib/architecture/format.ts';
	import { num } from '#lib/desk/format.ts';
	import type { DecisionDetail as Detail, MasterRecord } from '#lib/architecture/types.ts';
	import ConsensusBar from './ConsensusBar.svelte';
	import ContributionBars from './ContributionBars.svelte';
	import ReviewForm from './ReviewForm.svelte';
	import StopBanner from './StopBanner.svelte';

	let { decision, onmember }: { decision: Detail; onmember?: (id: string) => void } = $props();

	const c = $derived(decision.council);
	const m = $derived(decision.master);
	const full = $derived(m && 'messages' in m ? (m as MasterRecord) : null);
	const merr = $derived(m && 'error' in m ? m.error : null);
	const rows = $derived(executionRows(decision));
	const vetoed = $derived(!!c?.stop && c.stop[0].startsWith('sentinel_veto'));
	const councilStop = $derived(decision.stage === 'council' && decision.status === 'stopped');
	const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
	const mark = { ok: '✓', fail: '✕', skip: '–' } as const;
	const markCls = { ok: 'text-up', fail: 'text-down', skip: 'text-faint' } as const;
	const dot =
		'absolute -left-[22px] top-0.5 grid size-4 place-items-center rounded-full border bg-raise font-mono text-[9.5px]';
	const msgTone = { accent: 'text-accent', violet: 'text-violet', down: 'text-down', warn: 'text-warn', mute: 'text-mute' } as const;
</script>

<article class="grid gap-3" data-testid="decision-detail" data-id={decision.id}>
	<header class="flex flex-wrap items-center gap-2">
		<h2 class="text-sm font-semibold" data-testid="detail-title">
			{decision.symbol}
			{#if decision.side}{decision.side === 'bull' ? 'long' : 'short'}{/if}
			<span class="num ml-1 font-normal text-mute">{clock(decision.created_ms)}</span>
		</h2>
		<span class="pill border-violet text-violet">{decision.venue === 'alpaca' ? 'Alpaca paper' : 'MT5'}</span>
		<span
			class="pill {decision.status === 'traded'
				? 'border-up text-up'
				: decision.status === 'stopped'
					? 'border-down text-down'
					: 'border-warn text-warn'}"
		>
			{decision.status}
		</span>
		<span class="num ml-auto text-2xs text-faint" title={decision.id}>{decision.id}</span>
	</header>

	<StopBanner {decision}>
		{#if decision.status === 'waiting'}
			<div class="grid justify-items-end gap-1">
				<button
					type="button"
					class="btn btn-primary"
					disabled={architecture.asking}
					onclick={() => void architecture.ask(decision.id)}
					data-testid="ask-friday"
				>
					{architecture.asking ? 'Friday is thinking…' : 'Ask Friday'}
				</button>
				<span class="text-2xs text-faint">Paid Desk turn: counts against the Desk monthly cap.</span>
			</div>
		{/if}
	</StopBanner>

	{#if architecture.askError}
		<p class="border border-down/60 bg-down/10 px-3 py-2 text-xs text-down" role="alert">{architecture.askError}</p>
	{/if}
	{#if architecture.askNote}
		<p class="border border-accent/40 bg-accent/5 px-3 py-2 text-xs text-ink" role="status" data-testid="ask-reply">
			{architecture.askNote}
		</p>
	{/if}

	<ol
		class="relative grid gap-3 pl-[22px] before:absolute before:top-1.5 before:bottom-1.5 before:left-[7px] before:w-0.5 before:bg-line"
	>
		<!-- 1 · Signals -->
		<li class="relative" data-testid="step-signals">
			<span class="{dot} border-up text-up">1</span>
			<h3 class="mb-1.5 text-xs font-semibold">
				Signals <span class="font-normal text-faint">{clock(decision.created_ms)} {decision.trigger.kind}</span>
			</h3>
			<div class="flex flex-wrap gap-1 text-2xs">
				{#if decision.trigger.detail}<span class="border border-line-hi px-1.5 text-mute">{decision.trigger.detail}</span>{/if}
				{#if decision.price !== null}<span class="num border border-line-hi px-1.5 text-mute">price {decision.price}</span>{/if}
				{#if decision.atr !== null}<span class="num border border-line-hi px-1.5 text-mute">ATR {decision.atr}</span>{/if}
				{#if decision.weights_version !== null}<span class="num border border-line-hi px-1.5 text-mute">weights v{decision.weights_version}</span>{/if}
			</div>
		</li>

		<!-- 2 · Members -->
		<li class="relative" data-testid="step-members">
			<span class="{dot} {decision.step >= 1 ? 'border-up text-up' : 'border-down text-down'}">2</span>
			<h3 class="mb-1.5 text-xs font-semibold">
				General members
				<span class="font-normal text-faint">
					{decision.opinions.filter((o) => o.stance !== 'neutral').length} of {decision.opinions.length} took a side
				</span>
			</h3>
			<div class="grid grid-cols-[repeat(auto-fill,minmax(13rem,1fr))] gap-2">
				{#each decision.opinions as o (o.agent)}
					<div class="border border-line bg-deck p-2 text-xs" data-testid="opinion-{o.agent}">
						<div class="flex items-center justify-between">
							<button type="button" class="font-semibold hover:text-accent" onclick={() => onmember?.(o.agent)} aria-label="Open {cap(o.agent)} details">
								{cap(o.agent)}
							</button>
							<span class="num {stanceClass(o.stance)}">{stanceChip(o.stance, o.confidence)}</span>
						</div>
						<div class="my-1 h-1 bg-line" role="img" aria-label="Confidence {o.confidence} of 100">
							<span class="block h-full {stanceBg(o.stance)}" style="width:{o.confidence}%"></span>
						</div>
						<ul class="list-disc pl-3.5 text-2xs text-mute">
							{#each o.evidence as e, i (i)}<li>{e}</li>{/each}
						</ul>
						{#each o.sources as s, i (i)}
							<p class="mt-1 text-[10.5px] text-faint">{s.name}{s.at ? ` · ${clock(s.at)}` : ''}</p>
						{/each}
						<p class="num mt-0.5 text-[10.5px] text-faint">
							weight {o.weight.toFixed(2)} · adds {signed(o.contribution, 3)}{o.outcome ? ` · ${o.outcome}` : ''}
						</p>
						{#if o.data}
							<details class="mt-1">
								<summary class="cursor-pointer text-[10.5px] text-accent">Raw data</summary>
								<pre class="num mt-1 overflow-auto bg-void p-1.5 text-[10.5px] text-mute">{JSON.stringify(o.data, null, 2)}</pre>
							</details>
						{/if}
					</div>
				{/each}
			</div>
		</li>

		<!-- 3 · Council -->
		<li class="relative" data-testid="step-council">
			<span class="{dot} {c?.passed ? 'border-up text-up' : councilStop ? 'border-down text-down' : 'border-line-hi text-mute'}">3</span>
			<h3 class="mb-1.5 flex flex-wrap items-baseline gap-2 text-xs font-semibold">
				Council
				{#if c}
					<span class="num font-normal {c.passed ? 'text-up' : 'text-down'}">
						{signed(c.score)}
						{Math.abs(c.score) >= c.threshold ? '≥' : '<'}
						{c.threshold.toFixed(2)} · {c.non_neutral} non-neutral
					</span>
				{/if}
			</h3>
			{#if c}
				<div class="grid gap-2 sm:grid-cols-2">
					<div class="border border-line bg-deck p-2 text-xs">
						<p class="flex justify-between"><b class="text-up">Bull case</b><span class="num text-up">{signed(c.bull.total)}</span></p>
						<ul class="mt-1 list-disc pl-3.5 text-2xs text-mute">
							{#each c.bull.points as p (p.agent)}<li>{cap(p.agent)}: {p.evidence}</li>{:else}<li class="list-none text-faint">None</li>{/each}
						</ul>
					</div>
					<div class="border border-line bg-deck p-2 text-xs">
						<p class="flex justify-between"><b class="text-down">Bear case</b><span class="num text-down">{signed(c.bear.total)}</span></p>
						<ul class="mt-1 list-disc pl-3.5 text-2xs text-mute">
							{#each c.bear.points as p (p.agent)}<li>{cap(p.agent)}: {p.evidence}</li>{:else}<li class="list-none text-faint">None</li>{/each}
						</ul>
					</div>
				</div>
				<div class="mt-2 grid gap-3 border border-line bg-deck p-2 md:grid-cols-2">
					<div>
						<p class="mb-1.5 text-2xs text-faint">Consensus, weighted by confidence</p>
						<ConsensusBar items={decision.opinions} />
					</div>
					<div>
						<p class="mb-1.5 text-2xs text-faint">Contribution to the score</p>
						<ContributionBars contributions={c.contributions} score={c.score} threshold={c.threshold} />
					</div>
				</div>
				<div class="mt-2 border border-line bg-deck p-2 text-xs" data-testid="risk-seat">
					<p class="mb-1 flex items-center justify-between font-semibold">
						Risk seat · Sentinel
						<span class="pill {vetoed ? 'border-down text-down' : 'border-up text-up'}">{vetoed ? 'veto' : 'no veto'}</span>
					</p>
					{#each c.risk as r, i (i)}
						<div class="grid grid-cols-[1.25rem_1fr_auto] gap-2 border-b border-dashed border-line py-0.5 last:border-0">
							<span class={vetoed ? 'text-down' : 'text-up'} aria-hidden="true">{vetoed ? '✕' : '✓'}</span>
							<span>{r.rule}</span>
							<span class="num text-faint">{r.reason}</span>
						</div>
					{/each}
				</div>
			{:else}
				<p class="text-xs text-mute">The council did not run for this decision.</p>
			{/if}
		</li>

		<!-- 4 · Master -->
		<li class="relative" data-testid="step-master">
			<span
				class="{dot} {decision.plan
					? 'border-up text-up'
					: decision.stage === 'master' && decision.status === 'stopped'
						? 'border-down text-down'
						: 'border-line-hi text-mute'}">4</span
			>
			<h3 class="mb-1.5 flex flex-wrap items-center gap-2 text-xs font-semibold">
				Master · Friday
				{#if full}
					<span class="pill border-violet text-violet">{full.model}</span>
					<span class="num font-normal text-faint">{usageText(full.usage)}</span>
				{/if}
			</h3>
			{#if merr}
				<p class="border border-down/60 bg-down/10 px-3 py-2 text-xs text-down" role="alert">Friday's turn failed: {merr}</p>
			{:else if !full}
				<p class="text-xs text-mute">
					{decision.status === 'waiting'
						? 'Friday has not been asked. Click Ask Friday above to run the paid step.'
						: decision.step < 2
							? 'Not reached: the council did not pass.'
							: 'No Friday record for this decision.'}
				</p>
			{:else}
				{#if decision.plan}
					{@const p = decision.plan}
					<div class="border border-line bg-deck p-2 text-xs" data-testid="parsed-plan">
						<p class="num">
							<b>Plan:</b>
							{p.side}
							{p.volume}
							{p.symbol} @ {num(p.entry, 2)} · stop {num(p.stop, 2)} · target {num(p.target, 2)} · risk {p.risk_percent}%{p.risk_amount !== null
								? ` (${num(p.risk_amount, 2)} ${p.currency ?? ''})`
								: ''}
						</p>
						{#if p.sizing_error}<p class="mt-1 text-warn">{p.sizing_error}</p>{/if}
					</div>
				{/if}
				{#if decision.verdict}
					{@const v = decision.verdict}
					<div class="mt-2 border bg-deck p-2 text-xs {v.decision === 'approved' ? 'border-up/40' : 'border-down/40'}" data-testid="sentinel-verdict">
						<p class="mb-1 font-semibold {v.decision === 'approved' ? 'text-up' : 'text-down'}">Sentinel {v.decision}</p>
						<ul class="list-disc pl-3.5 text-2xs text-mute">
							{#each v.reasons as r, i (i)}<li>{r}</li>{/each}
							{#if v.decision === 'approved'}{#each v.warnings as w, i (i)}<li class="text-warn">{w}</li>{/each}{/if}
						</ul>
						{#if v.decision === 'vetoed' && v.rules.length}<p class="num mt-1 text-2xs text-faint">rules: {v.rules.join(', ')}</p>{/if}
					</div>
				{/if}
				<details class="mt-2" data-testid="exact-prompt">
					<summary class="cursor-pointer text-xs text-accent">Exact prompt &amp; inputs (system prompt hash {full.system_hash})</summary>
					<div class="mt-1 grid gap-1.5">
						<div class="border border-line bg-void p-2">
							<p class="num mb-0.5 text-[10.5px] text-violet">system</p>
							<pre class="num max-h-56 overflow-auto text-[11.5px] leading-relaxed whitespace-pre-wrap text-ink/80">{full.system}</pre>
						</div>
						{#each full.messages as msg, i (i)}
							{@const v = describeMessage(msg)}
							<div class="border border-line bg-void p-2">
								<p class="num mb-0.5 text-[10.5px] {msgTone[v.tone]}">{v.label}</p>
								<pre class="num max-h-56 overflow-auto text-[11.5px] leading-relaxed whitespace-pre-wrap text-ink/80">{v.body}</pre>
							</div>
						{/each}
					</div>
				</details>
				<details class="mt-1">
					<summary class="cursor-pointer text-xs text-accent">Raw reply</summary>
					<pre class="num mt-1 max-h-56 overflow-auto border border-line bg-void p-2 text-[11.5px] whitespace-pre-wrap text-ink/80">{full.reply}</pre>
				</details>
			{/if}
		</li>

		<!-- 5 · Execution -->
		<li class="relative" data-testid="step-execution">
			<span
				class="{dot} {decision.status === 'traded'
					? 'border-up text-up'
					: decision.stage === 'execution' && decision.status === 'stopped'
						? 'border-down text-down'
						: 'border-line-hi text-mute'}">5</span
			>
			<h3 class="mb-1.5 text-xs font-semibold">Execution</h3>
			<div class="border border-line bg-deck px-2 py-1 text-xs">
				{#each rows as r (r.id)}
					<div class="grid grid-cols-[1.25rem_1fr_auto] gap-2 border-b border-dashed border-line py-1 last:border-0" data-testid="exec-{r.id}" data-state={r.state}>
						<span class={markCls[r.state]} aria-hidden="true">{mark[r.state]}</span>
						<span>{r.label}</span>
						<span class="num text-faint">{r.note}</span>
					</div>
				{/each}
			</div>
			{#if decision.result_r !== null || decision.pnl !== null}
				<dl class="num mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
					<dt class="text-faint">Result</dt>
					<dd class={decision.result_r !== null && decision.result_r < 0 ? 'text-down' : 'text-up'}>
						{rText(decision.result_r)}{decision.pnl !== null ? ` · ${decision.pnl > 0 ? '+' : ''}${decision.pnl.toFixed(2)}` : ''}
					</dd>
					{#if decision.closed_at}<dt class="text-faint">Closed</dt><dd>{clock(decision.closed_at)}</dd>{/if}
				</dl>
			{/if}
		</li>
	</ol>

	<section class="border border-line bg-panel p-3" aria-labelledby="review-h">
		<h3 id="review-h" class="hud-title mb-2">Manual review · {STAGE_LABEL[decision.stage]}</h3>
		{#key decision.id}
			<ReviewForm decisionId={decision.id} review={decision.review} />
		{/key}
	</section>
</article>
