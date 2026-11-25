<script lang="ts">
	import { onMount, type Snippet } from 'svelte';
	import { totpOk } from '#lib/architecture/format.ts';

	let {
		title,
		children,
		confirmLabel = 'Confirm',
		needsTotp,
		error = null,
		busy = false,
		onconfirm,
		oncancel
	}: {
		title: string;
		children: Snippet;
		confirmLabel?: string;
		/** Show the authenticator code field (login is on). */
		needsTotp: boolean;
		error?: string | null;
		busy?: boolean;
		onconfirm: (totp: string) => void;
		oncancel: () => void;
	} = $props();

	let dlg = $state<HTMLDialogElement>();
	let totp = $state('');
	let local = $state<string | null>(null);
	const uid = $props.id();

	onMount(() => {
		const before = document.activeElement as HTMLElement | null;
		dlg?.showModal();
		return () => before?.focus?.();
	});

	function submit(e: SubmitEvent) {
		e.preventDefault();
		if (needsTotp && !totpOk(totp)) {
			local = 'Enter the 6-digit code from your authenticator app';
			return;
		}
		local = null;
		onconfirm(totp);
	}
</script>

<dialog
	bind:this={dlg}
	class="m-auto w-[min(26rem,calc(100%-2rem))] border border-line-hi bg-panel p-0 text-ink backdrop:bg-void/70"
	aria-labelledby="{uid}-t"
	oncancel={(e) => {
		e.preventDefault();
		if (!busy) oncancel();
	}}
	data-testid="confirm-step"
>
	<form class="grid gap-3 p-4 text-xs" onsubmit={submit}>
		<h2 id="{uid}-t" class="text-sm font-semibold">{title}</h2>
		<div class="grid gap-1.5 text-mute">{@render children()}</div>
		{#if needsTotp}
			<label class="grid gap-1 text-2xs text-faint">
				Authenticator code
				<input
					class="input num tracking-[0.4em]"
					type="text"
					inputmode="numeric"
					autocomplete="one-time-code"
					maxlength="6"
					placeholder="123456"
					bind:value={totp}
					oninput={(e) => {
						totp = e.currentTarget.value.replace(/\D/g, '').slice(0, 6);
						e.currentTarget.value = totp;
					}}
					data-testid="confirm-totp"
				/>
			</label>
		{/if}
		{#if local ?? error}<p class="text-down" role="alert" data-testid="confirm-error">{local ?? error}</p>{/if}
		<div class="flex justify-end gap-2">
			<button type="button" class="btn" onclick={oncancel} disabled={busy} data-testid="confirm-cancel">Cancel</button>
			<button type="submit" class="btn btn-primary" disabled={busy} data-testid="confirm-go">{busy ? 'Working…' : confirmLabel}</button>
		</div>
	</form>
</dialog>
