import { expect, test, type Page } from '@playwright/test';

// Assistant mode (tasks/spec-assistant-mode.md), in mock mode: the switch, the butler, 3D models
// in the sandbox, "remember that", and the sandbox holding when a model tries to escape.

async function openAssistant(page: Page, flags = 'reset') {
	await page.goto(`/?mock=${flags}&mode=assistant`);
	await expect(page.getByTestId('assistant-main')).toBeVisible();
}

async function ask(page: Page, text: string) {
	await page.getByRole('textbox', { name: /Message/ }).fill(text);
	await page.getByRole('button', { name: 'Send' }).click();
}

test('the mode switch moves between Trading and Assistant, each with its own chat', async ({ page }) => {
	await page.goto('/?mock=reset&mode=trading');
	const sw = page.getByTestId('mode-switch');
	await expect(sw.getByRole('button', { name: 'Trading' })).toHaveAttribute('aria-pressed', 'true');
	await expect(page.getByRole('group', { name: 'Persona' })).toBeVisible();

	await sw.getByRole('button', { name: 'Assistant' }).click();
	await expect(page.getByTestId('assistant-main')).toBeVisible();
	await expect(page.getByRole('group', { name: 'Persona' })).toHaveCount(0);
	await expect(page.getByTestId('brain-badge')).toContainText('claude-opus-5-5');
	await ask(page, 'hello');
	await expect(page.getByText('Very good, sir.')).toBeVisible();

	await sw.getByRole('button', { name: 'Trading' }).click();
	await expect(page.getByText('Very good, sir.')).toHaveCount(0);
	await expect(page.locator('#panel-watchlist')).toBeVisible();

	await sw.getByRole('button', { name: 'Assistant' }).click();
	await expect(page.getByText('Very good, sir.')).toBeVisible();
});

test('a 3D model renders in the sandbox and can be closed and reopened', async ({ page }) => {
	await openAssistant(page);
	await ask(page, 'Show me a 3D model of a jet engine');
	const stage = page.getByTestId('model-stage');
	await expect(stage).toBeVisible();
	await expect(stage.getByRole('heading')).toContainText('Jet engine');
	// The frame reports back: the triangle count only shows once the scene was built.
	await expect(stage.getByText(/tris/)).toBeVisible({ timeout: 15_000 });
	const frame = page.getByTestId('model-frame');
	await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');

	await stage.getByRole('button', { name: 'Close model' }).click();
	await expect(stage).toHaveCount(0);
	await page.getByTestId('model-chip').first().click();
	await expect(page.getByTestId('model-stage')).toBeVisible();
	await expect(page.getByTestId('model-stage').getByText(/tris/)).toBeVisible({ timeout: 15_000 });
});

test('a model that tries to escape the sandbox fails inside it and reaches nothing', async ({ page }) => {
	const apiCalls: string[] = [];
	page.on('request', (r) => {
		if (r.url().includes('/api/') && r.frame() !== page.mainFrame()) apiCalls.push(r.url());
	});
	await openAssistant(page);
	await ask(page, 'Show me the escape test model');
	const stage = page.getByTestId('model-stage');
	const alert = stage.getByRole('alert');
	await expect(alert).toBeVisible({ timeout: 15_000 });
	// Reading the app's page and storage both failed inside the sandbox.
	await expect(alert).toContainText('parent blocked');
	await expect(alert).toContainText('storage blocked');
	await page.waitForTimeout(500);
	expect(apiCalls).toEqual([]);
	// The app itself is untouched: the kill switch was not engaged.
	await expect(page.getByTestId('kill-banner')).toHaveCount(0);
});

test('changing a model makes a new one from the old', async ({ page }) => {
	await openAssistant(page);
	await ask(page, 'Show me a 3D model of a red cube');
	await expect(page.getByTestId('model-stage').getByRole('heading')).toContainText('Red cube');
	await ask(page, 'now make it blue');
	await expect(page.getByTestId('model-stage').getByRole('heading')).toContainText('Red cube (blue)');
	await expect(page.getByTestId('model-chip')).toHaveCount(2);
});

test('remember that creates a note and the galaxy shows it', async ({ page }) => {
	await openAssistant(page);
	await ask(page, 'Remember that the finish window should be 900 ms');
	await expect(page.getByText(/Noted, sir/)).toBeVisible();
	const chip = page.getByTestId('note-chip').first();
	await expect(chip).toContainText('The finish window should be 900');
	await chip.click();
	await expect(page).toHaveURL(/\/galaxy\?note=\d+/);
	await expect(page.getByRole('heading', { name: 'The finish window should be 900' })).toBeVisible();
});

test('without a key, Assistant mode says how to set it up', async ({ page }) => {
	await openAssistant(page, 'noclaude');
	await expect(page.getByTestId('assistant-notice')).toContainText('ANTHROPIC_API_KEY');
	await ask(page, 'hello');
	await expect(page.getByText(/needs Claude/)).toBeVisible();
});

test('asking for a trade in Assistant mode gets a refusal, never a preview', async ({ page }) => {
	await openAssistant(page);
	await ask(page, 'buy 0.1 gold');
	await expect(page.getByText(/no access to trading/)).toBeVisible();
	await expect(page.getByRole('dialog')).toHaveCount(0);
});

test.describe('at 375 px', () => {
	test.use({ viewport: { width: 375, height: 812 }, hasTouch: true });

	test('Assistant mode fits a phone, with a model on the stage', async ({ page }) => {
		await openAssistant(page);
		await ask(page, 'Show me the solar system in 3D');
		await expect(page.getByTestId('model-stage')).toBeVisible();
		const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
		expect(overflow).toBeLessThanOrEqual(0);
	});
});

test('a model that never stops does not freeze the app, and the watchdog closes it', async ({ page }) => {
	test.setTimeout(60_000);
	await openAssistant(page);
	await ask(page, 'Show me the endless model, run it forever');
	await expect(page.getByTestId('model-stage')).toBeVisible();
	// The app stays usable while the model's frame spins: switch modes and back.
	const sw = page.getByTestId('mode-switch');
	await sw.getByRole('button', { name: 'Trading' }).click();
	await expect(page.locator('#panel-watchlist')).toBeVisible();
	await sw.getByRole('button', { name: 'Assistant' }).click();
	await expect(page.getByTestId('model-stage').getByRole('alert')).toContainText('stopped responding', {
		timeout: 25_000
	});
});

test('a model cannot navigate its frame to another page', async ({ page }) => {
	await openAssistant(page);
	await ask(page, 'Show me a model that will wander off');
	// The code runs in a worker, where the location is read-only: the attempt fails.
	await expect(page.getByTestId('model-stage').getByRole('alert')).toContainText('failed', { timeout: 15_000 });
	const src = await page.getByTestId('model-frame').count();
	expect(src).toBe(0);
});

test('messages that do not come from the model frame are ignored', async ({ page }) => {
	await openAssistant(page);
	await ask(page, 'Show me a 3D model of a red cube');
	const stage = page.getByTestId('model-stage');
	await expect(stage.getByText(/tris/)).toBeVisible({ timeout: 15_000 });
	// A forged message on purpose: the test checks that the app ignores it.
	// nosemgrep: javascript.browser.security.wildcard-postmessage-configuration.wildcard-postmessage-configuration
	await page.evaluate(() => window.postMessage({ type: 'error', message: 'forged' }, '*'));
	await page.waitForTimeout(300);
	await expect(stage.getByRole('alert')).toHaveCount(0);
});

test('the sandbox page refuses to run outside a sandboxed frame', async ({ page }) => {
	await page.goto('/sandbox/model.html');
	await expect(page.locator('body')).toContainText('only runs inside Arc MT');
	await expect(page.locator('canvas')).toHaveCount(0);
});
