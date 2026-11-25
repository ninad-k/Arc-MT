import { expect, test, type Page } from '@playwright/test';
import { modal } from './helpers';

// Architecture page (tasks/spec-architecture-page.md), in mock mode: the pipeline map, the node
// drawer, the funnel and why-no-trade reasons, the exact prompt, the manual review and Ask Friday.

async function openPipeline(page: Page) {
	await page.goto('/architecture?mock=architecture');
	await expect(page.getByTestId('pipeline-map')).toBeVisible();
}

async function openDecisions(page: Page) {
	await page.goto('/architecture?mock=architecture&tab=decisions');
	await expect(page.getByTestId('decision-table')).toBeVisible();
	await expect(page.getByTestId('decision-row').first()).toBeVisible();
}

const row = (page: Page, symbol: string) => page.getByTestId('decision-row').filter({ hasText: symbol });

test('the map shows the six general members', async ({ page }) => {
	await openPipeline(page);
	await expect(page).toHaveTitle('Architecture · Arc MT');
	for (const id of ['atlas', 'capitol', 'scout', 'athena', 'chartist', 'oracle']) {
		await expect(page.getByTestId(`node-${id}`)).toBeVisible();
	}
	await expect(page.getByTestId('tier-members').getByRole('button')).toHaveCount(6);
	await expect(page.getByTestId('tab-pipeline')).toHaveAttribute('aria-current', 'page');
});

test('clicking Oracle opens the drawer and Escape closes it', async ({ page }) => {
	await openPipeline(page);
	const oracle = page.getByTestId('node-oracle');
	await oracle.click();
	const drawer = page.getByRole('dialog', { name: 'Oracle' });
	await expect(drawer).toBeVisible();
	await expect(drawer).toHaveAttribute('aria-modal', 'true');
	await expect(page.getByTestId('drawer-how')).toContainText('0.55');
	await page.keyboard.press('Escape');
	await expect(drawer).toHaveCount(0);
	await expect(oracle).toBeFocused();
});

test('the Decisions tab shows the funnel and why EURUSD did not trade', async ({ page }) => {
	await openDecisions(page);
	await expect(page.getByTestId('funnel')).toBeVisible();
	await expect(page.getByTestId('funnel-step-0')).toBeVisible();
	await expect(page.getByTestId('funnel-step-5')).toBeVisible();
	const eur = row(page, 'EURUSD');
	await expect(eur.getByTestId('decision-result')).toContainText(/below the 0\.30 threshold/);
	await eur.click();
	await expect(page).toHaveURL(/tab=decisions&d=dec_/);
	await expect(page.getByTestId('stop-banner')).toContainText('stopped at Council');
	await expect(page.getByTestId('stop-code')).toHaveText('council_below_threshold');
});

test('opening NVDA shows the exact prompt Friday was given', async ({ page }) => {
	await openDecisions(page);
	await row(page, 'NVDA').click();
	await expect(page.getByTestId('detail-title')).toContainText('NVDA');
	const prompt = page.getByTestId('exact-prompt');
	await expect(prompt).toBeVisible();
	await prompt.getByText(/Exact prompt/).click();
	await expect(prompt).toContainText('You are Friday');
	await expect(prompt).toContainText('tool result: ask_oracle');
	await expect(page.getByTestId('parsed-plan')).toContainText('NVDA');
});

test('saving a review works and a bad one shows the server error', async ({ page }) => {
	await openDecisions(page);
	await row(page, 'NVDA').click();
	const form = page.getByTestId('review-form');
	await form.getByRole('button', { name: 'Momentum continuation' }).click();
	await form.getByTestId('rating-4').click();
	await form.getByTestId('review-notes').fill('Target was too tight.');
	await form.getByTestId('review-save').click();
	await expect(form.getByTestId('review-saved')).toBeVisible();
	await expect(row(page, 'NVDA')).toContainText('Momentum continuation');

	await form.getByTestId('review-notes').fill('x'.repeat(4001).slice(0, 4000));
	await form.getByRole('button', { name: 'Add a mistake tag' }).fill('y'.repeat(70));
	await form.getByTestId('review-save').click();
	await expect(form.getByTestId('review-error')).toBeVisible();
});

test('Scan now adds a waiting decision, and Ask Friday goes to the usual confirmation', async ({ page }) => {
	await openDecisions(page);
	await page.getByTestId('scan-now').click();
	await expect(page.getByTestId('scan-note')).toBeVisible({ timeout: 15_000 });

	await row(page, 'USDJPY').click();
	await expect(page.getByTestId('stop-banner')).toContainText('Waiting');
	await page.getByTestId('ask-friday').click();
	const m = modal(page);
	await expect(m).toBeVisible({ timeout: 15_000 });
	await expect(m.getByTestId('preview-description')).toContainText(/Buy .* USDJPY at market/);
	await page.keyboard.press('Escape');
	await expect(m).toHaveCount(0);
});

test('no horizontal scroll on a phone', async ({ page }) => {
	await page.setViewportSize({ width: 390, height: 800 });
	await openPipeline(page);
	const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
	expect(overflow).toBeLessThanOrEqual(0);
});

async function openLearning(page: Page) {
	await page.goto('/architecture?mock=architecture&tab=learning');
	await expect(page.getByTestId('learning-tab')).toBeVisible();
	await expect(page.getByTestId('scorecard')).toBeVisible();
}

test('the Learning tab shows the scorecard, the guard table and a locked loosening', async ({ page }) => {
	await openLearning(page);
	await expect(page.getByTestId('score-oracle')).toContainText('0.17');
	await expect(page.getByTestId('score-oracle')).toContainText('+0.11');
	await expect(page.getByTestId('guard-table')).toContainText('Counterfactual move (ATR)');
	await expect(page.getByTestId('weight-proposal')).toContainText('Holdout');

	const loosen = page.locator('[data-direction="loosen"]');
	await expect(loosen.getByTestId('direction-badge')).toHaveText('loosen');
	await expect(loosen.getByTestId('locked-reason')).toContainText('loosening needs 100 closed decisions, there are 9');
	await expect(loosen.getByRole('button', { name: /Apply/ })).toBeDisabled();
	await expect(page.getByTestId('rejected')).toBeVisible();
	await expect(page.getByTestId('versions')).toContainText('v7');
});

test('a tightening applies in one click and makes a new version', async ({ page }) => {
	await openLearning(page);
	const tighten = page.locator('[data-direction="tighten"]');
	await tighten.getByRole('button', { name: 'Apply', exact: true }).click();
	await expect(tighten.getByTestId('applied')).toContainText('Applied as v8');
	await expect(page.getByTestId('versions')).toContainText('v8');
});

test('a rollback that loosens a guard asks for a second confirm', async ({ page }) => {
	await openLearning(page);
	await page.getByTestId('rollback-5').click();
	const step = page.getByTestId('confirm-step');
	await expect(step).toBeVisible();
	await expect(step).toContainText('min_council_score');
	await step.getByTestId('confirm-go').click();
	await expect(page.getByTestId('confirm-step')).toHaveCount(0);
	await expect(page.getByTestId('versions')).toContainText('Rollback to v5');
});

test('the lock chip shows the masked account and opens its drawer', async ({ page }) => {
	await openPipeline(page);
	const chip = page.getByTestId('lock-chip');
	await expect(chip).toContainText('••7904');
	await expect(chip).toContainText('locked');
	await chip.click();
	const drawer = page.getByRole('dialog', { name: 'Master account lock' });
	await expect(drawer).toBeVisible();
	await expect(drawer.getByTestId('lock-account')).toHaveText('••7904');
	await drawer.getByTestId('lock-show').click();
	await expect(drawer.getByTestId('lock-account')).toHaveText('53107904');

	await drawer.getByTestId('lock-new').fill('12345678');
	await drawer.getByTestId('lock-again').fill('12345679');
	await drawer.getByTestId('lock-save').click();
	await expect(drawer.getByTestId('lock-error')).toContainText('differ');

	await page.keyboard.press('Escape');
	await expect(drawer).toHaveCount(0);
	await expect(chip).toBeFocused();
});
