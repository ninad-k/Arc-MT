<script lang="ts">
	import { architecture } from '#lib/state/architecture.svelte.ts';
	import type { Review, ReviewInput } from '#lib/architecture/types.ts';

	let { decisionId, review }: { decisionId: string; review: Review | null } = $props();

	const PRESETS = {
		setup: ['Momentum continuation', 'Breakout', 'Mean reversion'],
		mistakes: ['Late entry', 'Ignored event', 'Target too tight'],
		regime: ['Trend', 'Chop', 'High vol']
	};
	const start = () => review;

	let setup = $state<string | null>(start()?.setup ?? null);
	let mistakes = $state<string[]>([...(start()?.mistakes ?? [])]);
	let regime = $state<string | null>(start()?.regime ?? null);
	let rating = $state<number | null>(start()?.rating ?? null);
	let takeAgain = $state<boolean | null>(start()?.take_again ?? null);
	let notes = $state(start()?.notes ?? '');
	let draft = $state({ setup: '', mistakes: '', regime: '' });
	let saved = $state(false);

	const withSelected = (presets: string[], sel: (string | null)[]) => [...presets, ...sel.filter((t): t is string => !!t && !presets.includes(t))];
	const setupTags = $derived(withSelected(PRESETS.setup, [setup]));
	const mistakeTags = $derived(withSelected(PRESETS.mistakes, mistakes));
	const regimeTags = $derived(withSelected(PRESETS.regime, [regime]));

	function addTag(kind: 'setup' | 'mistakes' | 'regime') {
		const t = draft[kind].trim();
		if (!t) return;
		draft[kind] = '';
		if (kind === 'setup') setup = t;
		else if (kind === 'regime') regime = t;
		else if (!mistakes.includes(t)) mistakes = [...mistakes, t];
	}
	const toggleMistake = (t: string) => (mistakes = mistakes.includes(t) ? mistakes.filter((m) => m !== t) : [...mistakes, t]);

	async function save() {
		const body: ReviewInput = {
			setup,
			mistakes,
			regime,
			rating,
			take_again: takeAgain,
			notes: notes.trim() === '' ? null : notes
		};
		saved = await architecture.saveReview(decisionId, body);
	}

	const chip = (on: boolean) =>
		`border px-1.5 text-2xs transition-colors ${on ? 'border-accent text-accent' : 'border-line-hi text-mute hover:text-ink'}`;
</script>

{#snippet adder(kind: 'setup' | 'mistakes' | 'regime', label: string)}
	<span class="inline-flex">
		<input
			class="num h-5 w-24 border border-line bg-void/60 px-1 text-2xs text-ink focus-visible:border-accent"
			aria-label="Add a {label} tag"
			placeholder="+ add"
			maxlength="64"
			bind:value={draft[kind]}
			onkeydown={(e) => {
				if (e.key === 'Enter') {
					e.preventDefault();
					addTag(kind);
				}
			}}
			onblur={() => addTag(kind)}
		/>
	</span>
{/snippet}

<form
	class="grid gap-3"
	data-testid="review-form"
	onsubmit={(e) => {
		e.preventDefault();
		void save();
	}}
>
	<fieldset class="grid gap-1">
		<legend class="mb-1 text-2xs text-faint">Setup</legend>
		<div class="flex flex-wrap gap-1">
			{#each setupTags as t (t)}
				<button type="button" class={chip(setup === t)} aria-pressed={setup === t} onclick={() => (setup = setup === t ? null : t)}>{t}</button>
			{/each}
			{@render adder('setup', 'setup')}
		</div>
	</fieldset>
	<fieldset class="grid gap-1">
		<legend class="mb-1 text-2xs text-faint">Mistakes</legend>
		<div class="flex flex-wrap gap-1">
			{#each mistakeTags as t (t)}
				<button type="button" class={chip(mistakes.includes(t))} aria-pressed={mistakes.includes(t)} onclick={() => toggleMistake(t)}>{t}</button>
			{/each}
			{@render adder('mistakes', 'mistake')}
		</div>
	</fieldset>
	<fieldset class="grid gap-1">
		<legend class="mb-1 text-2xs text-faint">Regime</legend>
		<div class="flex flex-wrap gap-1">
			{#each regimeTags as t (t)}
				<button type="button" class={chip(regime === t)} aria-pressed={regime === t} onclick={() => (regime = regime === t ? null : t)}>{t}</button>
			{/each}
			{@render adder('regime', 'regime')}
		</div>
	</fieldset>

	<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
		<div role="radiogroup" aria-label="Rating" class="flex items-center gap-0.5">
			<span class="mr-1 text-2xs text-faint">Rating</span>
			{#each [1, 2, 3, 4, 5] as n (n)}
				<button
					type="button"
					role="radio"
					aria-checked={rating === n}
					aria-label="{n} of 5"
					class="px-0.5 text-lg leading-none {rating !== null && n <= rating ? 'text-warn' : 'text-line-hi hover:text-mute'}"
					onclick={() => (rating = rating === n ? null : n)}
					data-testid="rating-{n}"
				>
					★
				</button>
			{/each}
		</div>
		<div role="radiogroup" aria-label="Take again?" class="flex items-center gap-1">
			<span class="mr-1 text-2xs text-faint">Take again?</span>
			<button type="button" role="radio" aria-checked={takeAgain === true} class={chip(takeAgain === true)} onclick={() => (takeAgain = takeAgain === true ? null : true)}>Yes</button>
			<button type="button" role="radio" aria-checked={takeAgain === false} class={chip(takeAgain === false)} onclick={() => (takeAgain = takeAgain === false ? null : false)}>No</button>
		</div>
	</div>

	<label class="grid gap-1 text-2xs text-faint">
		Notes
		<textarea
			class="min-h-20 w-full border border-line-hi bg-deck px-2 py-1.5 text-xs text-ink focus-visible:border-accent"
			maxlength="4000"
			bind:value={notes}
			data-testid="review-notes"
		></textarea>
	</label>

	<div class="flex flex-wrap items-center gap-3">
		<button type="submit" class="btn btn-primary" disabled={architecture.reviewSaving} data-testid="review-save">
			{architecture.reviewSaving ? 'Saving…' : 'Save review'}
		</button>
		{#if architecture.reviewError}
			<span class="text-xs text-down" role="alert" data-testid="review-error">{architecture.reviewError}</span>
		{:else if saved}
			<span class="text-xs text-up" role="status" data-testid="review-saved">Review saved</span>
		{/if}
	</div>
</form>
