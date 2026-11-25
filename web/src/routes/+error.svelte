<script lang="ts">
	// Every error SvelteKit catches lands here: an unknown route, a failed load, and a page that
	// throws while rendering, even after it mounted. hooks.client.ts has already logged it.
	import { page } from '$app/state';
	import CrashScreen from '#lib/components/CrashScreen.svelte';
	import { crashMessage } from '#lib/crash.ts';
</script>

<CrashScreen status={page.status} message={crashMessage(page.error, page.status)} />
