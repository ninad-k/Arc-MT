<script lang="ts">
	import { uploads } from '#lib/state/uploads.svelte.ts';

	let input = $state<HTMLInputElement>();

	function pick(e: Event) {
		const el = e.currentTarget as HTMLInputElement;
		const file = el.files?.[0];
		el.value = ''; // the same file can be picked again
		if (file) void uploads.open(file);
	}
</script>

<input
	bind:this={input}
	type="file"
	class="sr-only"
	tabindex="-1"
	accept=".glb,.gltf,.obj,.stl,.ply,image/png,image/jpeg,image/webp"
	data-testid="upload-input"
	onchange={pick}
/>
<button
	type="button"
	class="grid size-10 shrink-0 place-items-center border border-line-hi bg-void/70 text-mute transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
	aria-label="Upload a 3D file or a picture"
	title="Upload a 3D file (.glb, .gltf, .obj, .stl, .ply) or a picture"
	disabled={uploads.busy !== null}
	onclick={() => input?.click()}
	data-testid="upload-button"
>
	<svg viewBox="0 0 24 24" class="size-4" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
		<path d="M21 11.5 12.6 19.9a5.5 5.5 0 0 1-7.8-7.8l8.5-8.5a3.7 3.7 0 0 1 5.2 5.2l-8.5 8.5a1.8 1.8 0 0 1-2.6-2.6l7.8-7.8" stroke-linecap="round" stroke-linejoin="round" />
	</svg>
</button>
