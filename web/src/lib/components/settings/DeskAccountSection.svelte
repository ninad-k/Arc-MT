<script lang="ts">
	import LockControl from '#lib/components/architecture/LockControl.svelte';
</script>

<!-- Not a SettingsSection: that wraps its body in a form, and the lock control has its own. -->
<section id="desk" aria-labelledby="desk-title" data-testid="section-desk" class="hud scroll-mt-4 p-4 sm:p-5">
	<h2 id="desk-title" class="font-mono text-sm font-semibold tracking-[0.18em] text-ink uppercase">Desk master account</h2>
	<p class="mt-1.5 max-w-prose text-xs text-mute">
		The Desk master bot is locked to one MT5 account. The same control is on the Architecture page, behind the lock chip.
	</p>
	<div class="mt-4"><LockControl /></div>
</section>
