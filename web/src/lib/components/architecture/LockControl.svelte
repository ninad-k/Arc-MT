<script lang="ts">
	import { onMount } from 'svelte';
	import { lockTone, maskAccount } from '#lib/architecture/format.ts';
	import { learning } from '#lib/state/learning.svelte.ts';

	const view = $derived(learning.lock);
	let shown = $state(false);
	let expected = $state('');
	let again = $state('');
	let broker = $state('');
	let totp = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);
	let ok = $state(false);
	const uid = $props.id();

	onMount(() => {
		void learning.loadLock();
		void learning.loadAuth();
	});

	const needsCode = $derived(learning.authEnabled !== false);
	const toneCls = { up: 'text-up', down: 'text-down', warn: 'text-warn', mute: 'text-mute' } as const;
	const icon = { locked: '✓', mismatch: '✕', unverified: '?', off: '–' } as const;
	const digits = (s: string) => s.replace(/\D/g, '');

	async function save(e: SubmitEvent) {
		e.preventDefault();
		error = null;
		ok = false;
		if (expected === '') {
			error = 'Enter the account number, digits only (0 turns the lock off)';
			return;
		}
		if (expected !== again) {
			error = 'The two numbers differ';
			return;
		}
		if (needsCode && !/^\d{6}$/.test(totp)) {
			error = 'Enter the 6-digit code from your authenticator app';
			return;
		}
		busy = true;
		const r = await learning.saveLock({
			expected: Number(expected),
			confirm: Number(again),
			...(broker.trim() ? { broker: broker.trim() } : {}),
			...(totp ? { totp } : {})
		});
		busy = false;
		if (r.ok) {
			ok = true;
			expected = again = totp = '';
		} else error = r.message;
	}
</script>

<div class="grid gap-3 text-xs" data-testid="lock-control">
	{#if learning.lockError}
		<p class="text-down" role="alert">{learning.lockError}</p>
	{:else if !view}
		<div class="skeleton h-20" aria-busy="true" aria-label="Loading the account lock"></div>
	{:else}
		<div class="border border-line bg-deck p-3">
			<h3 class="hud-title mb-1.5">Status</h3>
			<p class="text-base {toneCls[lockTone(view.state)]}" data-testid="lock-state">{icon[view.state]} {view.state}</p>
			<p class="mt-1 text-mute" data-testid="lock-message">{view.message}</p>
		</div>
		<div class="border border-line bg-deck p-3">
			<h3 class="hud-title mb-1.5">Expected account</h3>
			<dl class="num grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-1">
				<dt class="text-faint">Broker</dt>
				<dd>{view.broker}</dd>
				<dt class="text-faint">Account</dt>
				<dd class="flex items-center gap-2">
					<span data-testid="lock-account">{shown ? view.expected : view.masked || maskAccount(view.expected)}</span>
					<button type="button" class="btn h-6 px-2" aria-pressed={shown} onclick={() => (shown = !shown)} data-testid="lock-show">
						{shown ? 'Hide' : 'Show'}
					</button>
				</dd>
			</dl>
		</div>
	{/if}

	<form class="grid gap-2 border border-line bg-deck p-3" onsubmit={save}>
		<h3 class="hud-title">Change the master account</h3>
		<label class="text-2xs text-faint" for="{uid}-a">Change to</label>
		<input id="{uid}-a" class="input num" inputmode="numeric" autocomplete="off" placeholder="account number" bind:value={expected} oninput={() => (expected = digits(expected))} data-testid="lock-new" />
		<label class="text-2xs text-faint" for="{uid}-b">Type it again to confirm</label>
		<input id="{uid}-b" class="input num" inputmode="numeric" autocomplete="off" bind:value={again} oninput={() => (again = digits(again))} data-testid="lock-again" />
		<label class="text-2xs text-faint" for="{uid}-c">Broker label (optional)</label>
		<input id="{uid}-c" class="input" placeholder="IC Markets" maxlength="40" bind:value={broker} data-testid="lock-broker" />
		{#if needsCode}
			<label class="text-2xs text-faint" for="{uid}-d">Authenticator code</label>
			<input
				id="{uid}-d"
				class="input num tracking-[0.4em]"
				inputmode="numeric"
				autocomplete="one-time-code"
				maxlength="6"
				placeholder="123456"
				bind:value={totp}
				oninput={() => (totp = digits(totp).slice(0, 6))}
				data-testid="lock-totp"
			/>
		{/if}
		<div class="flex flex-wrap items-center gap-3">
			<button type="submit" class="btn btn-primary" disabled={busy} data-testid="lock-save">{busy ? 'Saving…' : 'Save'}</button>
			{#if error}<span class="text-down" role="alert" data-testid="lock-error">{error}</span>{/if}
			{#if ok}<span class="text-up" role="status" data-testid="lock-saved">Saved</span>{/if}
		</div>
		<p class="text-2xs text-faint">
			No MT5 password is needed: MT5 is already logged in. Enter 0 to turn the lock off. The Desk refuses MT5 trades on a mismatch,
			or when the EA is older than 0.7.0 and cannot report the account.
		</p>
	</form>
</div>
