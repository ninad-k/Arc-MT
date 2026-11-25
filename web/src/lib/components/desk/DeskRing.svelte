<script lang="ts">
	import Orb from '#lib/components/Orb.svelte';
	import { desk } from '#lib/state/desk.svelte.ts';
	import { ringPosition, seatTone } from '#lib/desk/format.ts';
	import type { Seat } from '#lib/desk/types.ts';

	/** The nine specialists around Friday's orb, by group, clockwise from the top. */
	const ORDER = ['atlas', 'capitol', 'scout', 'athena', 'chartist', 'oracle', 'sentinel', 'pilot', 'ledger'] as const;

	const seats = $derived(
		ORDER.map((id) => desk.state?.seats?.find((s) => s.id === id)).filter((s): s is Seat => s !== undefined)
	);

	const toneClass: Record<ReturnType<typeof seatTone>, string> = {
		accent: 'border-accent text-accent shadow-[0_0_18px_-4px_var(--color-accent)]',
		up: 'border-up/60 text-up',
		down: 'border-down/70 text-down',
		mute: 'border-line text-mute',
		faint: 'border-line/60 text-faint opacity-60'
	};
	const groupLabel: Record<string, string> = {
		intelligence: 'Intel',
		analysis: 'Analysis',
		execution: 'Execution'
	};
</script>

<div class="hud relative h-full min-h-0 w-full overflow-hidden" data-testid="desk-ring" aria-label="Desk seats">
	<!-- The ring the seats sit on. -->
	<svg class="pointer-events-none absolute inset-0 h-full w-full" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
		<ellipse cx="50" cy="50" rx="38" ry="38" fill="none" stroke="var(--color-line-hi)" stroke-width="0.25" stroke-dasharray="0.8 1.6" />
	</svg>
	<div class="absolute inset-[20%]">
		<Orb readouts={false} />
	</div>
	<ul class="absolute inset-0">
		{#each seats as seat, i (seat.id)}
			{@const pos = ringPosition(i, seats.length, 38)}
			{@const tone = seatTone(seat.state)}
			<li
				class="absolute w-[6.25rem] -translate-x-1/2 -translate-y-1/2 border bg-panel/85 px-2 py-1 backdrop-blur-sm transition-colors max-sm:w-[5.25rem] {toneClass[tone]}"
				style="left:{pos.x}%;top:{pos.y}%"
				title={seat.summary ?? `${seat.title}: ${seat.state}`}
				data-testid="seat-{seat.id}"
				data-state={seat.state}
			>
				<div class="flex items-center justify-between gap-1">
					<span class="truncate text-2xs font-semibold tracking-wider uppercase">{seat.id}</span>
					<span
						class="size-1.5 shrink-0 rounded-full bg-current {seat.state === 'working' ? 'motion-safe:animate-[pulse-dot_1s_ease-in-out_infinite]' : ''}"
						aria-hidden="true"
					></span>
				</div>
				<div class="truncate text-[0.625rem] text-faint normal-case">
					{seat.title} · {groupLabel[seat.group] ?? seat.group}
				</div>
				<span class="sr-only">{seat.state}</span>
			</li>
		{/each}
	</ul>
</div>
