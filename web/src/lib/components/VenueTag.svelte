<script lang="ts">
	import type { Venue } from '#lib/api.ts';

	/** Marks rows that are not on MT5 (spec-alpaca). MT5 rows show nothing, as before. */
	let { venue }: { venue?: Venue } = $props();
</script>

{#if venue === 'alpaca'}
	<span
		class="pill h-5 border-violet/50 px-1.5 text-violet"
		title="Alpaca paper account (US stocks)"
		data-testid="venue-tag">Alpaca</span
	>
{/if}
