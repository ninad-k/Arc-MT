<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { api, type NoteNode, type NoteText, type NotesGraph } from '#lib/api.ts';
	import { describeError, toApiError } from '#lib/errors.ts';
	import { graphKey, layout, neighbours } from '#lib/galaxy/layout.ts';
	import { parseNote, type Span } from '#lib/galaxy/markdown.ts';
	import type { GalaxyScene } from '#lib/galaxy/GalaxyScene.ts';
	import { chat } from '#lib/state/chat.svelte.ts';
	import { speech } from '#lib/state/speech.svelte.ts';
	import { persona } from '#lib/state/persona.svelte.ts';

	/** Group colours: distinct on black, readable as text. */
	const PALETTE = ['#7dd3fc', '#fbbf24', '#a78bfa', '#2ee6a6', '#f472b6', '#fb923c', '#60a5fa', '#e879f9', '#a3e635', '#f87171'];
	const POSITIONS_KEY = 'jarvis.galaxy.positions';

	let graph = $state<NotesGraph | null>(null);
	let loadError = $state<string | null>(null);
	let notesOff = $state(false);
	let rescanning = $state(false);
	let fallback = $state(false);

	let host = $state<HTMLDivElement>();
	let scene: GalaxyScene | undefined;
	/** Bumped on every (re)load, so a slow earlier load never mounts a second scene. */
	let generation = 0;
	/** The note text request in flight, aborted when another note is opened. */
	let noteRequest: AbortController | null = null;

	let selected = $state<NoteNode | null>(null);
	let text = $state<NoteText | null>(null);
	let textError = $state<string | null>(null);
	let search = $state('');
	let question = $state('');
	let answer = $state<{ text: string; persona: string } | null>(null);

	const groups = $derived.by(() => {
		const counts = new Map<string, number>();
		for (const n of graph?.nodes ?? []) counts.set(n.group, (counts.get(n.group) ?? 0) + 1);
		return [...counts.entries()].sort((a, b) => a[0].localeCompare(b[0])).map(([name, count], i) => ({ name, count, color: PALETTE[i % PALETTE.length] }));
	});
	const colors = $derived(new Map(groups.map((g) => [g.name, g.color])));
	const blocks = $derived(text ? parseNote(text.text) : []);
	const matches = $derived.by(() => {
		const q = search.trim().toLowerCase();
		if (!q || !graph) return null;
		return graph.nodes.filter((n) => n.title.toLowerCase().includes(q) || n.excerpt.toLowerCase().includes(q));
	});

	function positionsFor(g: NotesGraph): Float32Array {
		const key = graphKey(g.nodes, g.links);
		try {
			const saved = JSON.parse(localStorage.getItem(POSITIONS_KEY) ?? 'null') as { key: string; p: number[] } | null;
			if (saved?.key === key && saved.p.length === g.nodes.length * 3) return Float32Array.from(saved.p);
		} catch {
			/* recompute */
		}
		const p = layout(g.nodes, g.links);
		try {
			localStorage.setItem(POSITIONS_KEY, JSON.stringify({ key, p: [...p] }));
		} catch {
			/* not cached */
		}
		return p;
	}

	async function mountScene(g: NotesGraph, gen: number) {
		scene?.dispose();
		scene = undefined;
		if (!host || g.nodes.length === 0) return;
		try {
			const { GalaxyScene } = await import('#lib/galaxy/GalaxyScene.ts');
			if (!host || gen !== generation) return; // a newer load took over
			scene = new GalaxyScene(host, {
				nodes: g.nodes,
				links: g.links,
				positions: positionsFor(g),
				colors,
				reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
				onSelect: (id) => (id === null ? clear() : void open(id)),
				onContextLost: () => (fallback = true)
			});
			scene.start();
			if (selected) scene.select(selected.id, neighbours(selected.id, g.links));
		} catch (e) {
			console.warn('Galaxy: WebGL unavailable', e);
			fallback = true;
		}
	}

	async function load(rescan = false) {
		loadError = null;
		const gen = ++generation;
		try {
			const g = rescan ? await api.rescanNotes() : await api.notesGraph();
			if (gen !== generation) return;
			graph = g;
			// ?note=<id> opens that note, with or without 3D.
			const raw = page.url.searchParams.get('note');
			if (!rescan && raw && /^\d+$/.test(raw)) void open(Number(raw), false);
			await Promise.resolve(); // let the canvas host render
			await mountScene(g, gen);
		} catch (e) {
			const err = toApiError(e);
			if (err.status === 404) notesOff = true;
			else loadError = describeError(err);
		}
	}

	async function rescan() {
		rescanning = true;
		try {
			await load(true);
		} finally {
			rescanning = false;
		}
	}

	async function open(id: number, fly = true) {
		const node = graph?.nodes.find((n) => n.id === id) ?? null;
		if (!node || !graph) return;
		selected = node;
		text = null;
		textError = null;
		scene?.select(id, neighbours(id, graph.links), fly);
		noteRequest?.abort();
		const ctrl = new AbortController();
		noteRequest = ctrl;
		try {
			const loaded = await api.note(id, ctrl.signal);
			// Another note may have been opened meanwhile: show only the one still selected.
			if (selected?.id === id && !ctrl.signal.aborted) text = loaded;
		} catch (e) {
			const err = toApiError(e);
			if (err.code !== 'aborted' && selected?.id === id) textError = describeError(err);
		}
	}

	function clear() {
		noteRequest?.abort();
		selected = null;
		text = null;
		scene?.select(null);
	}

	function openByTitle(title: string) {
		const want = title.toLowerCase().replace(/[-_]/g, ' ').trim();
		const n = graph?.nodes.find(
			(x) => x.title.toLowerCase().replace(/[-_]/g, ' ') === want || (x.path.split('/').pop() ?? '').replace(/\.md$/i, '').toLowerCase().replace(/[-_]/g, ' ') === want
		);
		if (n) void open(n.id);
	}

	$effect(() => {
		const m = matches;
		scene?.setHighlight(m ? new Set(m.map((n) => n.id)) : null);
	});

	let panelWidth = $state(0);
	$effect(() => {
		// On wide screens the panel covers the right side: keep the selected star visible left of
		// it. On a phone the panel is a bottom sheet and covers nothing on the side.
		const covered = selected && window.matchMedia('(min-width: 640px)').matches ? panelWidth : 0;
		scene?.setInsetRight(covered);
	});

	async function ask(ev: SubmitEvent) {
		ev.preventDefault();
		const q = question.trim();
		if (!q || chat.pending) return;
		question = '';
		answer = null;
		const res = await chat.send(q);
		if (!res) {
			answer = { text: 'The question could not be answered. See the transcript on the HUD.', persona: persona.name };
			return;
		}
		answer = { text: res.reply, persona: persona.nameFor(res.persona) };
		if (speech.readAloud && speech.available)
			void speech.speak(res.reply, res.mode === 'assistant' ? res.persona : undefined);
		// "Remember that ..." (Assistant mode) wrote a note: rebuild the galaxy so the new star is
		// born, then fly to it.
		const captured = res.captured ?? [];
		if (captured.length > 0) {
			await load(true);
			void open(captured[captured.length - 1].id);
			return;
		}
		const cited = res.notes ?? [];
		// Small talk and "not in my notes" move nothing; one to three notes: the first one;
		// four or more: the whole cluster.
		if (cited.length >= 4) scene?.flyToCluster(cited.map((c) => c.id));
		else if (cited.length > 0) void open(cited[0].id);
	}

	function onKey(ev: KeyboardEvent) {
		if (ev.key === 'Escape' && selected) clear();
	}

	onMount(() => {
		void load();
		return () => {
			scene?.dispose();
			scene = undefined;
		};
	});

	const spanClass = (s: Span) =>
		s.kind === 'bold' ? 'font-semibold text-ink' : s.kind === 'code' ? 'num bg-raise px-1 text-2xs text-accent' : '';
</script>

<svelte:head><title>Notes galaxy · Arc MT</title></svelte:head>
<svelte:window onkeydown={onKey} />

<div class="flex h-dvh flex-col bg-void">
	<header
		class="z-10 flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-line bg-deck/80 px-4 pt-[max(0.5rem,env(safe-area-inset-top))] pb-2 backdrop-blur"
	>
		<a href="/" class="num text-xs text-mute hover:text-ink" data-testid="back-to-hud">← HUD</a>
		<h1 class="font-mono text-sm font-semibold tracking-[0.2em] uppercase">Notes galaxy</h1>
		{#if graph}
			<span class="num text-2xs text-faint" data-testid="note-count">{graph.nodes.length} notes · {graph.links.length} links</span>
		{/if}
		<div class="ml-auto flex items-center gap-2">
			<label class="sr-only" for="galaxy-search">Search notes</label>
			<input
				id="galaxy-search"
				type="search"
				bind:value={search}
				placeholder="Search notes"
				class="h-8 w-44 border border-line-hi bg-void/70 px-2 text-sm text-ink placeholder:text-faint focus-visible:border-accent focus-visible:outline-none max-sm:w-32"
				onkeydown={(e) => {
					if (e.key === 'Enter' && matches?.length) void open(matches[0].id);
				}}
			/>
			<button
				type="button"
				class="num h-8 border border-line-hi px-2.5 text-2xs text-mute hover:border-accent hover:text-ink disabled:opacity-40"
				onclick={() => void rescan()}
				disabled={rescanning || notesOff}
			>
				{rescanning ? 'Rescanning…' : 'Rescan'}
			</button>
			<button
				type="button"
				class="num h-8 border border-line-hi px-2.5 text-2xs text-mute hover:border-accent hover:text-ink"
				onclick={() => {
					clear();
					search = '';
					scene?.reset();
				}}
			>
				Overview
			</button>
		</div>
	</header>

	<div class="relative min-h-0 flex-1">
		<div bind:this={host} class="galaxy absolute inset-0" role="img" aria-label="3D map of your notes. Click a star to open the note." data-testid="galaxy-canvas"></div>

		{#if notesOff}
			<div class="absolute inset-0 grid place-items-center p-6 text-center">
				<p class="max-w-md text-sm text-mute">Notes are turned off on this server. Set <code class="num text-accent">[notes] enabled = true</code> in <code class="num">config/jarvis.toml</code> and restart.</p>
			</div>
		{:else if loadError}
			<div class="absolute inset-0 grid place-items-center p-6 text-center" role="alert">
				<p class="max-w-md text-sm text-down">Could not load the notes. {loadError}</p>
			</div>
		{:else if graph && graph.nodes.length === 0}
			<div class="absolute inset-0 grid place-items-center p-6 text-center" data-testid="galaxy-empty">
				<div class="max-w-md space-y-2 text-sm text-mute">
					{#if graph.error}
						<p>{graph.error}.</p>
					{:else}
						<p>No notes yet in <code class="num text-accent">{graph.dir}</code>.</p>
					{/if}
					<p>Put markdown (<code class="num">.md</code>) files in that folder, or set <code class="num text-accent">[notes] dir</code> in the config. Sub-folders become colours; link notes with <code class="num">[[Title]]</code>. Sample notes are in <code class="num">examples/notes</code>.</p>
				</div>
			</div>
		{:else if fallback}
			<div class="absolute inset-0 grid place-items-center p-6 text-center">
				<p class="max-w-md text-sm text-mute">3D graphics are not available in this browser. Use the search box to find a note.</p>
			</div>
		{/if}

		{#if groups.length}
			<ul class="pointer-events-none absolute top-3 left-4 space-y-1 text-2xs" aria-label="Folders">
				{#each groups as g (g.name)}
					<li class="num flex items-center gap-2 text-mute">
						<span class="size-2 rounded-full" style="background:{g.color};box-shadow:0 0 8px {g.color}" aria-hidden="true"></span>
						{g.name} <span class="text-faint">{g.count}</span>
					</li>
				{/each}
			</ul>
		{/if}

		{#if matches}
			<ul class="absolute top-3 right-4 max-h-64 w-64 overflow-auto border border-line bg-panel/90 text-sm backdrop-blur max-sm:hidden" aria-label="Matching notes">
				{#each matches.slice(0, 12) as n (n.id)}
					<li><button type="button" class="block w-full px-3 py-1.5 text-left text-mute hover:bg-raise hover:text-ink" onclick={() => void open(n.id)}>{n.title}</button></li>
				{:else}
					<li class="px-3 py-1.5 text-faint">No note matches.</li>
				{/each}
			</ul>
		{/if}

		{#if selected}
			<aside
				class="absolute top-0 right-0 bottom-0 flex w-[min(26rem,100%)] flex-col border-l border-line bg-panel/95 backdrop-blur max-sm:top-auto max-sm:h-[55%] max-sm:w-full max-sm:border-t max-sm:border-l-0"
				aria-label="Note"
				data-testid="note-panel"
				bind:clientWidth={panelWidth}
			>
				<div class="flex items-start gap-2 border-b border-line px-4 py-3">
					<div class="min-w-0 flex-1">
						<h2 class="text-base font-semibold break-words text-ink">{selected.title}</h2>
						<p class="num truncate text-2xs text-faint">{selected.path}</p>
					</div>
					<button type="button" class="num px-1 text-mute hover:text-ink" aria-label="Close note" onclick={clear}>✕</button>
				</div>
				<div class="min-h-0 flex-1 space-y-3 overflow-auto px-4 py-3 text-sm leading-relaxed text-mute">
					{#if textError}
						<p class="text-down">{textError}</p>
					{:else if !text}
						<p class="text-faint">{selected.excerpt}</p>
					{:else}
						{#each blocks as b, i (i)}
							{#if b.kind === 'heading'}
								{#if b.level > 1}
									<h3 class="pt-1 text-xs font-semibold tracking-wide text-ink uppercase">{#each b.spans as s, j (j)}{s.text}{/each}</h3>
								{/if}
							{:else if b.kind === 'paragraph'}
								<p>{#each b.spans as s, j (j)}{#if s.kind === 'link'}<button type="button" class="text-accent underline decoration-dotted underline-offset-2 hover:text-ink" onclick={() => openByTitle(s.target)}>{s.text}</button>{:else}<span class={spanClass(s)}>{s.text}</span>{/if}{/each}</p>
							{:else}
								<svelte:element this={b.ordered ? 'ol' : 'ul'} class="space-y-1 pl-5 {b.ordered ? 'list-decimal' : 'list-disc'}">
									{#each b.items as item, k (k)}
										<li>{#each item as s, j (j)}{#if s.kind === 'link'}<button type="button" class="text-accent underline decoration-dotted underline-offset-2 hover:text-ink" onclick={() => openByTitle(s.target)}>{s.text}</button>{:else}<span class={spanClass(s)}>{s.text}</span>{/if}{/each}</li>
									{/each}
								</svelte:element>
							{/if}
						{/each}
					{/if}
				</div>
			</aside>
		{/if}

		<div class="pointer-events-none absolute right-4 bottom-4 left-4 flex flex-col items-center gap-2 {selected ? 'sm:right-[min(27rem,100%)]' : ''}">
			{#if answer || chat.pending}
				<div class="pointer-events-auto w-full max-w-2xl border-l-2 border-accent bg-panel/90 px-3 py-2 text-sm text-ink backdrop-blur" aria-live="polite" data-testid="galaxy-answer">
					{#if chat.pending}
						<span class="text-mute">{persona.name} is thinking…</span>
					{:else if answer}
						<span class="sr-only">{answer.persona}: </span>{answer.text}
					{/if}
				</div>
			{/if}
			<form class="pointer-events-auto flex w-full max-w-2xl gap-2" onsubmit={ask}>
				<label class="sr-only" for="galaxy-ask">Ask about your notes</label>
				<input
					id="galaxy-ask"
					bind:value={question}
					placeholder="Ask {persona.name} about your notes…"
					maxlength="2000"
					class="h-10 min-w-0 flex-1 border border-line-hi bg-void/80 px-3 text-sm text-ink placeholder:text-faint backdrop-blur focus-visible:border-accent focus-visible:outline-none"
				/>
				<button type="submit" class="num h-10 border border-accent px-4 text-2xs tracking-widest text-accent uppercase hover:bg-accent/10 disabled:opacity-40" disabled={chat.pending || !question.trim()}>Ask</button>
			</form>
		</div>
	</div>
</div>

<style>
	.galaxy :global(.galaxy-label) {
		font: 500 12px/1.2 ui-monospace, monospace;
		color: var(--color-ink);
		background: color-mix(in srgb, var(--color-panel) 85%, transparent);
		border: 1px solid var(--color-line-hi);
		padding: 2px 6px;
	}
</style>
