<script lang="ts">
	/**
	 * Shown by `+error.svelte` instead of a screen that crashed or could not load. Says plainly that
	 * the server, and so trading, is not affected.
	 */
	let { message, status }: { message: string; status?: number } = $props();
</script>

<main class="grid min-h-dvh place-items-center overflow-y-auto p-4">
	<section class="hud w-full max-w-md space-y-4 p-5" role="alert" aria-labelledby="crash-title">
		<h1 id="crash-title" class="font-mono text-sm font-semibold tracking-[0.2em] text-down">
			{status && status !== 500 ? `ERROR ${status}` : 'THIS SCREEN STOPPED'}
		</h1>
		<p class="text-sm text-ink">{message}</p>
		<p class="text-xs text-mute">
			The Arc MT server keeps running: open positions, your limits and the kill switch are not
			affected. Details are in the browser console.
		</p>
		<div class="flex gap-2">
			{#if status !== 404}
				<button
					type="button"
					onclick={() => location.reload()}
					class="h-10 flex-1 border border-accent bg-accent/15 text-xs font-semibold tracking-widest text-accent uppercase transition-colors hover:bg-accent/25"
				>
					Try again
				</button>
			{/if}
			<a
				href="/"
				class="grid h-10 flex-1 place-items-center border border-line-hi text-xs font-semibold tracking-widest text-mute uppercase transition-colors hover:text-ink"
			>
				Home
			</a>
		</div>
	</section>
</main>
