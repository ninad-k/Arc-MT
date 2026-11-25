<script lang="ts">
	import { onMount } from 'svelte';
	import {
		DASH,
		ciBar,
		clock,
		directionBadge,
		gateProgress,
		rate,
		relativeTime,
		signed,
		stopLabel,
		weightMoves
	} from '#lib/architecture/format.ts';
	import type { ReviewItem, Weights } from '#lib/architecture/types.ts';
	import { learning } from '#lib/state/learning.svelte.ts';
	import ConfirmStep from './ConfirmStep.svelte';

	let { onmember }: { onmember?: (id: string) => void } = $props();

	onMount(() => {
		void learning.load();
		void learning.loadAuth();
	});

	const d = $derived(learning.data);
	const snap = $derived(d?.snapshot ?? null);
	const gates = $derived(snap?.gates ?? null);
	const review = $derived(d?.reviews[0] ?? null);
	const items = $derived((review?.items ?? []).filter((i) => !learning.dismissed.includes(i.id)));
	const needsCode = $derived(learning.authEnabled !== false);
	const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
	const w2 = (v: number) => v.toFixed(2);
	const num2 = (v: number | null | undefined) => (typeof v === 'number' ? v.toFixed(2) : DASH);
	const budget = $derived(d?.spend?.budget_usd ?? 2);
	const spent = $derived(d?.spend?.spent_usd ?? 0);

	// ---- Suggestions: apply, and the second step for a loosening ----
	let busyItem = $state<string | null>(null);
	let itemErrors = $state<Record<string, string>>({});
	let loosen = $state<{ reviewId: string; item: ReviewItem } | null>(null);
	let loosenError = $state<string | null>(null);
	let loosenBusy = $state(false);

	async function apply(item: ReviewItem) {
		if (!review) return;
		if (item.direction === 'loosen') {
			loosenError = null;
			loosen = { reviewId: review.id, item };
			return;
		}
		busyItem = item.id;
		const r = await learning.apply(review.id, item);
		busyItem = null;
		itemErrors = { ...itemErrors, [item.id]: r.ok ? '' : r.message };
	}

	async function confirmLoosen(totp: string) {
		if (!loosen) return;
		loosenBusy = true;
		const r = await learning.apply(loosen.reviewId, loosen.item, { confirm_loosen: true, totp });
		loosenBusy = false;
		if (r.ok) loosen = null;
		else loosenError = r.message;
	}

	// ---- Versions: rollback, with a second step when the server says it would loosen a guard ----
	let rbBusy = $state<number | null>(null);
	let rbError = $state<string | null>(null);
	let rbConfirm = $state<{ version: number; message: string } | null>(null);
	let rbConfirmError = $state<string | null>(null);

	async function rollback(version: number) {
		rbError = null;
		rbBusy = version;
		const r = await learning.rollback(version);
		rbBusy = null;
		if (r.ok) return;
		if (r.status === 400) {
			rbConfirmError = null;
			rbConfirm = { version, message: r.message };
		} else rbError = r.message;
	}

	async function confirmRollback(totp: string) {
		if (!rbConfirm) return;
		rbBusy = rbConfirm.version;
		const r = await learning.rollback(rbConfirm.version, { confirm_loosen: true, totp });
		rbBusy = null;
		if (r.ok) rbConfirm = null;
		else rbConfirmError = r.message;
	}

	const isWeights = (v: unknown): v is Weights => typeof v === 'object' && v !== null;
	const moveCls = (v: number | null) => (v === null ? 'text-faint' : v > 0.005 ? 'text-warn' : v < -0.005 ? 'text-up' : 'text-mute');
	const moveNote = (v: number | null) => (v === null ? '' : v > 0.005 ? 'missed a move' : v < -0.005 ? 'avoided a move' : 'flat');
	const lastRun = $derived(gates?.last_review_ms ? relativeTime(gates.last_review_ms) : 'never');
</script>

<div class="grid gap-3" data-testid="learning-tab">
	{#if learning.error}
		<div class="flex items-center justify-between gap-3 border border-down/60 bg-down/10 px-3 py-2 text-xs text-down" role="alert">
			<span>{learning.error}</span>
			<button type="button" class="underline" onclick={() => void learning.load()}>Retry</button>
		</div>
	{/if}

	{#if !snap}
		{#if !learning.error}<div class="skeleton h-64" aria-busy="true" aria-label="Loading the learning data"></div>{/if}
	{:else}
		<div class="grid gap-3 xl:grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)]">
			<!-- Member scorecard -->
			<section class="hud p-3" aria-labelledby="lr-score">
				<h2 id="lr-score" class="hud-title mb-2 flex flex-wrap items-center gap-2">
					Member scorecard
					<span class="ml-auto text-faint normal-case tracking-normal">taken and counterfactual shown separately</span>
				</h2>
				<div class="overflow-x-auto">
					<table class="w-full border-collapse text-xs" data-testid="scorecard">
						<thead>
							<tr class="text-left text-[10.5px] tracking-wider text-faint uppercase">
								<th scope="col" class="px-2 py-1.5 font-medium">Member</th>
								<th scope="col" class="px-2 py-1.5 font-medium">n</th>
								<th scope="col" class="px-2 py-1.5 font-medium">Hit rate</th>
								<th scope="col" class="px-2 py-1.5 font-medium">Skill (80% CI)</th>
								<th scope="col" class="px-2 py-1.5 font-medium">Weight now → proposed</th>
							</tr>
						</thead>
						<tbody>
							{#each snap.members as m (m.agent)}
								{@const g = ciBar(m.skill, m.skill_low, m.skill_high)}
								{@const now = snap.proposal.current[m.agent]}
								{@const next = snap.proposal.proposed[m.agent]}
								<tr class="border-t border-line/60" data-testid="score-{m.agent}">
									<td class="px-2 py-1.5">
										<button type="button" class="font-semibold hover:text-accent" onclick={() => onmember?.(m.agent)} aria-label="Open {cap(m.agent)} details">{cap(m.agent)}</button>
									</td>
									<td class="num px-2 py-1.5" title="taken {m.n_taken ?? DASH} · counterfactual {m.n_counterfactual ?? DASH}">
										{m.n ?? DASH}{#if m.n !== null && m.n < 40}<span class="ml-1 text-warn" title="low sample" aria-label="low sample">•</span>{/if}
									</td>
									<td class="num px-2 py-1.5" title="taken {rate(m.hit_rate_taken)} · counterfactual {rate(m.hit_rate_counterfactual)}">{rate(m.hit_rate)}</td>
									<td class="px-2 py-1.5">
										{#if g}
											<span class="flex items-center gap-2">
												<span class="relative block h-2.5 w-36 border border-line bg-deck" role="img" aria-label="Skill {signed(m.skill)}{g.hasRange ? `, interval ${signed(m.skill_low)} to ${signed(m.skill_high)}` : ''}">
													<span class="absolute -top-0.5 -bottom-0.5 left-1/2 border-l border-line-hi"></span>
													{#if g.hasRange}<span class="absolute top-0.5 h-1 rounded-sm bg-accent/45" style="left:{g.rangeLeft}%;width:{g.rangeWidth}%"></span>{/if}
													<span class="absolute -top-px h-2.5 w-0.5 bg-accent" style="left:{g.marker}%"></span>
												</span>
												<span class="num {(m.skill ?? 0) > 0 ? 'text-up' : 'text-down'}">{signed(m.skill)}</span>
											</span>
										{:else}
											<span class="num text-faint">{DASH}</span>
										{/if}
									</td>
									<td class="num px-2 py-1.5 whitespace-nowrap">
										{w2(now)} →
										<span class={next > now + 0.0005 ? 'text-up' : next < now - 0.0005 ? 'text-down' : 'text-mute'}>{w2(next)}</span>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</section>

			<!-- Guards -->
			<section class="hud p-3" aria-labelledby="lr-guards">
				<h2 id="lr-guards" class="hud-title mb-2">Guards · what they stopped</h2>
				<div class="overflow-x-auto">
					<table class="w-full border-collapse text-xs" data-testid="guard-table">
						<thead>
							<tr class="text-left text-[10.5px] tracking-wider text-faint uppercase">
								<th scope="col" class="px-2 py-1.5 font-medium">Guard</th>
								<th scope="col" class="px-2 py-1.5 font-medium">Stops</th>
								<th scope="col" class="px-2 py-1.5 font-medium">Counterfactual move (ATR)</th>
							</tr>
						</thead>
						<tbody>
							{#each snap.guards as g (g.code)}
								<tr class="border-t border-line/60">
									<td class="px-2 py-1.5">
										<span class="block">{stopLabel(g.code)}</span>
										<span class="num text-2xs text-faint">{g.code}</span>
									</td>
									<td class="num px-2 py-1.5">{g.stops ?? DASH}<span class="text-faint"> / {g.resolved ?? DASH} resolved</span></td>
									<td class="num px-2 py-1.5 {moveCls(g.mean_move_atr)}">
										{signed(g.mean_move_atr)} <span class="text-faint">{moveNote(g.mean_move_atr)}</span>
									</td>
								</tr>
							{:else}
								<tr><td colspan="3" class="p-3 text-center text-mute">No guard has stopped a decision yet.</td></tr>
							{/each}
						</tbody>
					</table>
				</div>
				<p class="mt-2 text-2xs text-faint">
					The move in the council's direction after the stop, in ATR. Positive: the stop missed a move. Negative: it avoided one. There is no fill model, so treat it as indicative.
				</p>
			</section>
		</div>

		<!-- Weight proposal -->
		<section class="hud p-3" aria-labelledby="lr-prop" data-testid="weight-proposal">
			<h2 id="lr-prop" class="hud-title mb-2 flex flex-wrap items-center gap-2">
				Weight proposal (computed in code)
				<span class="pill {snap.proposal.eligible ? 'border-up text-up' : 'border-line-hi text-mute'}">{snap.proposal.eligible ? 'eligible' : 'not eligible yet'}</span>
			</h2>
			{#if snap.proposal.changes.length > 0}
				<ul class="num flex flex-wrap gap-x-4 gap-y-1 text-xs">
					{#each snap.proposal.changes as c (c.agent)}
						<li>{cap(c.agent)} <s class="text-down">{w2(c.old)}</s> → <ins class="text-up no-underline">{w2(c.new)}</ins> <span class="text-faint">· skill {signed(c.skill)} · n {c.n ?? DASH}</span></li>
					{/each}
				</ul>
			{:else}
				<p class="text-xs text-mute">No weight changes proposed.</p>
			{/if}
			{#if snap.proposal.holdout}
				<p class="num mt-2 text-xs text-mute" data-testid="holdout">
					Holdout (70/30, {snap.proposal.holdout.decisions} decisions): Brier {num2(snap.proposal.holdout.brier_old)} → {num2(snap.proposal.holdout.brier_new)}
					{#if snap.proposal.holdout.brier_old !== null && snap.proposal.holdout.brier_new !== null}
						<span class={snap.proposal.holdout.brier_new <= snap.proposal.holdout.brier_old ? 'text-up' : 'text-down'}>
							({snap.proposal.holdout.brier_new <= snap.proposal.holdout.brier_old ? 'not worse' : 'worse'})
						</span>
					{/if}
				</p>
			{/if}
			{#if snap.proposal.notes.length}
				<ul class="mt-2 list-disc pl-4 text-2xs text-faint">
					{#each snap.proposal.notes as n, i (i)}<li>{n}</li>{/each}
				</ul>
			{/if}
		</section>

		<!-- Suggestions from Claude -->
		<section class="hud p-3" aria-labelledby="lr-sugg">
			<h2 id="lr-sugg" class="hud-title mb-2 flex flex-wrap items-center gap-2">
				Suggestions · Claude review
				{#if d?.brain?.model}<span class="pill border-violet text-violet normal-case tracking-normal">{d.brain.model}</span>{/if}
				<span class="ml-auto flex flex-wrap items-center gap-3 normal-case tracking-normal">
					<span class="text-faint" data-testid="review-cost">last run {lastRun} · ~${(d?.estimate_usd ?? 0.03).toFixed(2)} · ${spent.toFixed(2)} of ${budget.toFixed(2)} this month</span>
					<button
						type="button"
						class="btn btn-primary"
						disabled={!gates?.can_review || learning.running || !!d?.brain_error}
						onclick={() => void learning.runReview()}
						data-testid="run-review"
					>
						{learning.running ? 'Reviewing…' : 'Run review'}
					</button>
				</span>
			</h2>
			{#if d?.brain_error}
				<p class="mb-2 text-xs text-warn" role="status" data-testid="brain-error">{d.brain_error}</p>
			{:else if gates && !gates.can_review && gates.why_not}
				<p class="mb-2 text-xs text-warn" role="status" data-testid="why-not">{gates.why_not}</p>
			{/if}
			{#if learning.runError}<p class="mb-2 text-xs text-down" role="alert">{learning.runError}</p>{/if}

			{#if !review}
				<p class="text-xs text-mute">No review yet. Claude reads the statistics above and suggests changes; code checks every number, and nothing applies until you click.</p>
			{:else}
				<p class="mb-2 text-xs text-mute" data-testid="review-summary">
					{review.summary} <span class="text-faint">· {clock(review.created_at)} · ${review.cost_usd.toFixed(3)}</span>
				</p>
				<div class="grid grid-cols-[repeat(auto-fill,minmax(19rem,1fr))] gap-3">
					{#each items as it (it.id)}
						{@const badge = directionBadge(it.direction)}
						<article class="grid content-start gap-1.5 border bg-deck p-3 text-xs {it.direction === 'loosen' ? 'border-warn/45' : 'border-line'}" data-testid="suggestion-{it.id}" data-direction={it.direction}>
							<div class="flex items-center gap-1.5">
								<span class="pill border-line-hi text-mute">{it.kind}</span>
								<span class="pill {badge.cls}" data-testid="direction-badge">{badge.label}</span>
								<span class="num ml-auto text-faint">conf {it.confidence}</span>
							</div>
							<p class="num" data-testid="diff">
								{#if isWeights(it.old) && isWeights(it.new)}
									{#each weightMoves(it.old, it.new) as mv, i (mv.agent)}{i > 0 ? ' · ' : ''}{cap(mv.agent)} <s class="text-down">{w2(mv.old)}</s> → <ins class="text-up no-underline">{w2(mv.new)}</ins>{/each}
								{:else}
									{it.param} <s class="text-down">{String(it.old)}</s> → <ins class="text-up no-underline">{String(it.new)}</ins>
								{/if}
							</p>
							<p class="text-mute">{it.rationale}</p>
							{#if it.evidence_refs.length}<p class="text-2xs text-faint">Evidence: {it.evidence_refs.join(' · ')}</p>{/if}
							{#if it.locked}<p class="text-2xs text-warn" data-testid="locked-reason">{it.locked}</p>{/if}
							{#if itemErrors[it.id]}<p class="text-down" role="alert" data-testid="apply-error">{itemErrors[it.id]}</p>{/if}
							<div class="flex gap-2">
								{#if it.applied_version !== null}
									<span class="num text-up" data-testid="applied">Applied as v{it.applied_version}</span>
								{:else}
									<button
										type="button"
										class="btn {it.locked ? '' : 'btn-primary'} h-8"
										disabled={!!it.locked || busyItem === it.id}
										title={it.locked ?? undefined}
										onclick={() => void apply(it)}
										data-testid="apply-{it.id}"
									>
										{busyItem === it.id ? 'Applying…' : it.locked ? 'Apply · locked' : 'Apply'}
									</button>
									<button type="button" class="btn h-8 border-transparent" onclick={() => learning.dismiss(it.id)} data-testid="dismiss-{it.id}">Dismiss</button>
								{/if}
							</div>
						</article>
					{:else}
						<p class="text-xs text-mute">All suggestions from this review are dismissed or applied.</p>
					{/each}
				</div>
				{#if review.rejected.length}
					<details class="mt-3" data-testid="rejected">
						<summary class="cursor-pointer text-xs text-accent">Rejected by the checks ({review.rejected.length})</summary>
						<ul class="mt-1 grid gap-1.5 text-xs">
							{#each review.rejected as r, i (i)}
								<li class="border border-line bg-deck p-2">
									<span class="text-down">{r.reason}</span>
									<code class="num mt-0.5 block text-2xs break-all text-faint">{JSON.stringify(r.item)}</code>
								</li>
							{/each}
						</ul>
					</details>
				{/if}
			{/if}
		</section>

		<div class="grid gap-3 lg:grid-cols-2">
			<!-- Versions -->
			<section class="hud p-3" aria-labelledby="lr-ver">
				<h2 id="lr-ver" class="hud-title mb-2">Config versions</h2>
				{#if rbError}<p class="mb-2 text-xs text-down" role="alert">{rbError}</p>{/if}
				{#if learning.versionsError}<p class="mb-2 text-xs text-down" role="alert">{learning.versionsError}</p>{/if}
				<div class="overflow-x-auto">
					<table class="w-full border-collapse text-xs" data-testid="versions">
						<thead>
							<tr class="text-left text-[10.5px] tracking-wider text-faint uppercase">
								<th scope="col" class="px-2 py-1.5 font-medium">Version</th>
								<th scope="col" class="px-2 py-1.5 font-medium">When</th>
								<th scope="col" class="px-2 py-1.5 font-medium">Change</th>
								<th scope="col" class="px-2 py-1.5 font-medium"><span class="sr-only">Action</span></th>
							</tr>
						</thead>
						<tbody>
							{#each learning.versions?.versions ?? [] as v (v.version)}
								{@const live = v.version === learning.versions?.live.version}
								<tr class="border-t border-line/60" data-testid="version-{v.version}">
									<td class="num px-2 py-1.5 whitespace-nowrap">v{v.version} {#if live}<span class="pill h-[18px] border-up text-up">live</span>{/if}</td>
									<td class="num px-2 py-1.5 whitespace-nowrap text-mute">{new Date(v.created_at).toLocaleDateString('en-GB', { day: '2-digit', month: 'short' })}</td>
									<td class="px-2 py-1.5">{v.reason ?? DASH}{#if v.actor}<span class="text-faint"> · {v.actor}</span>{/if}</td>
									<td class="px-2 py-1.5 text-right">
										{#if !live}
											<button type="button" class="btn h-7" disabled={rbBusy === v.version} onclick={() => void rollback(v.version)} data-testid="rollback-{v.version}">
												{rbBusy === v.version ? 'Rolling back…' : `Roll back to v${v.version}`}
											</button>
										{/if}
									</td>
								</tr>
							{:else}
								<tr><td colspan="4" class="p-3 text-center text-mute">No versions yet: the council uses the config defaults.</td></tr>
							{/each}
						</tbody>
					</table>
				</div>
			</section>

			<!-- Gates -->
			<section class="hud p-3" aria-labelledby="lr-gates">
				<h2 id="lr-gates" class="hud-title mb-2">Before a proposal is allowed</h2>
				{#if gates}
					{@const rows = [
						{ label: 'Closed decisions for a guard tightening', have: gates.decisions_closed, need: gates.min_decisions_tighten },
						{ label: 'Closed decisions for a guard loosening', have: gates.decisions_closed, need: gates.min_decisions_loosen },
						{ label: 'New resolved opinions since the last review', have: gates.resolved_since_last_review, need: gates.review_every_resolved }
					]}
					<div class="grid gap-2.5 text-xs" data-testid="gates">
						{#each rows as r (r.label)}
							{@const p = gateProgress(r.have, r.need)}
							<div>
								<p class="flex justify-between"><span>{r.label}</span><span class="num {p.done ? 'text-up' : 'text-mute'}">{r.have} of {r.need}</span></p>
								<div class="h-1.5 overflow-hidden bg-line" role="progressbar" aria-label={r.label} aria-valuemin="0" aria-valuemax={r.need} aria-valuenow={Math.min(r.have, r.need)}>
									<span class="block h-full {p.done ? 'bg-up' : 'bg-warn'}" style="width:{p.pct}%"></span>
								</div>
							</div>
						{/each}
						<p class="text-2xs text-faint">
							A loosening also needs {gates.min_guard_resolved_loosen} resolved decisions for that guard, a holdout pass, a second confirm and, when login is on, an authenticator code.
							Weights move at most ±0.05 per review. Code computes every number; Claude only explains and ranks, and has no write access.
						</p>
					</div>
				{/if}
			</section>
		</div>
	{/if}
</div>

{#if loosen}
	<ConfirmStep
		title="Loosen a guard"
		confirmLabel="Loosen"
		needsTotp={needsCode}
		error={loosenError}
		busy={loosenBusy}
		onconfirm={(t) => void confirmLoosen(t)}
		oncancel={() => (loosen = null)}
	>
		<p>
			This makes the Desk's risk checks <b class="text-warn">looser</b>: {loosen.item.param}
			{String(loosen.item.old)} → {String(loosen.item.new)}.
		</p>
		<p>It takes effect on the next decision. You can roll back from the version list.</p>
	</ConfirmStep>
{/if}

{#if rbConfirm}
	<ConfirmStep
		title="Roll back to v{rbConfirm.version}"
		confirmLabel="Roll back"
		needsTotp={needsCode}
		error={rbConfirmError}
		busy={rbBusy !== null}
		onconfirm={(t) => void confirmRollback(t)}
		oncancel={() => (rbConfirm = null)}
	>
		<p>{rbConfirm.message}</p>
		<p>This would make a guard looser. Confirm to roll back anyway.</p>
	</ConfirmStep>
{/if}
