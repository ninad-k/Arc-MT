import { expect, test, type Page } from '@playwright/test';
import { modal, openHud, say } from './helpers';

const panel = (page: Page) => page.getByRole('region', { name: 'Market view' });

/** Open the Market view panel (a tab below lg, a button above) and analyze a symbol. */
async function analyze(page: Page, symbol: string): Promise<void> {
	const tab = page.getByRole('tab', { name: 'Market', exact: true });
	if (await tab.isVisible()) await tab.click();
	else await page.getByRole('button', { name: 'Market view', exact: true }).click();
	await expect(panel(page)).toBeVisible();
	await panel(page).getByLabel('Symbol', { exact: true }).fill(symbol);
	await panel(page).getByRole('button', { name: 'Analyze', exact: true }).click();
}

test('the panel starts empty, with the disclaimer in view', async ({ page }) => {
	await openHud(page);
	await page.getByRole('button', { name: 'Market view', exact: true }).click();
	await expect(page.getByTestId('market-empty')).toBeVisible();
	await expect(page.getByTestId('market-disclaimer')).toContainText('not financial advice');
	// Nothing is analyzed until asked.
	await expect(page.getByTestId('market-card')).toHaveCount(0);
});

test('analyze XAUUSD: bullish card with a setup, factors and the disclaimer', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'XAUUSD');

	const live = page.getByTestId('market-live');
	const card = page.getByTestId('market-card');
	await expect(card).toBeVisible();
	await expect(live).toContainText('XAUUSD: Bullish, moderate confidence (62 of 100)');
	await expect(card.getByTestId('bias-pill')).toContainText('Bullish');
	await expect(card.getByTestId('confidence-text')).toContainText('62 / 100');
	await expect(card.getByTestId('stand-aside')).toHaveCount(0);

	const setup = card.getByTestId('market-setup');
	await expect(setup).toContainText('Entry zone');
	await expect(setup).toContainText('Stop loss');
	await expect(setup).toContainText('TP1');
	await expect(setup).toContainText('TP2');
	await expect(setup).toContainText('1.50');
	await expect(panel(page).getByRole('button', { name: 'Prepare trade' })).toBeVisible();

	await expect(card.getByTestId('tf-row')).toHaveCount(5);
	await expect(card.getByTestId('event-row').first()).toContainText('12 min ago');
	await expect(card.getByTestId('factor-row').first()).toContainText('H4 trend');
	await expect(card.getByTestId('valid-until')).toContainText(/expires in 3 h \d+ min|expires in 4 h/);
	await expect(card.getByTestId('market-summary')).not.toBeEmpty();
	await expect(page.getByTestId('market-disclaimer')).toContainText('not financial advice');

	// Headlines open in a new tab and say where they come from.
	await expect(card.getByTestId('news-label')).toContainText('Slightly positive (7 headlines)');
	const link = card.getByRole('link').first();
	await expect(link).toHaveAttribute('target', '_blank');
	await expect(link).toHaveAttribute('rel', 'noopener noreferrer');
	await expect(card.getByTestId('market-news')).toContainText('Example Wire');
	await expect(card.getByTestId('market-news').getByRole('link')).toHaveCount(5);
});

test('analyze EURUSD: stand-aside banner with the reason and no Prepare trade', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'EURUSD');

	const card = page.getByTestId('market-card');
	await expect(card).toBeVisible();
	await expect(card.getByTestId('stand-aside')).toContainText('Stand aside');
	await expect(card.getByTestId('stand-aside')).toContainText('High-impact USD event in 20 minutes');
	await expect(card.getByTestId('bias-pill')).toContainText('Bearish');
	await expect(card.getByTestId('market-setup')).toHaveCount(0);
	await expect(panel(page).getByRole('button', { name: 'Prepare trade' })).toHaveCount(0);
	await expect(card.getByTestId('event-row').first()).toContainText(/in (19|20) min/);
	await expect(card.getByTestId('market-news')).toContainText('Not configured');
});

test('analyze US100: neutral, no setup and no Prepare trade', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'US100');

	const card = page.getByTestId('market-card');
	await expect(card.getByTestId('bias-pill')).toContainText('Neutral');
	await expect(card.getByTestId('no-setup')).toBeVisible();
	await expect(card.getByTestId('stand-aside')).toHaveCount(0);
	await expect(panel(page).getByRole('button', { name: 'Prepare trade' })).toHaveCount(0);
});

test('an unknown symbol shows an error with a way to retry', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'NOSUCH');
	await expect(page.getByTestId('market-error')).toContainText('unknown symbol');
	await expect(page.getByRole('button', { name: 'Try again' })).toBeVisible();
});

test('Prepare trade: an empty volume is blocked, 0.01 opens the confirmation modal', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'XAUUSD');
	await panel(page).getByRole('button', { name: 'Prepare trade' }).click();

	const dlg = page.getByTestId('prepare-dialog');
	await expect(dlg).toBeVisible();
	// No default lot size, and the setup is restated.
	await expect(dlg.getByLabel('Volume (lots)')).toHaveValue('');
	await expect(page.getByTestId('prepare-setup-line')).toContainText(/Buy · stop [\d,.]+ · TP1 [\d,.]+/);
	await expect(dlg.getByTestId('order-type-market')).toBeChecked();
	await expect(dlg.getByText(/^Market · /)).toBeVisible();
	await expect(dlg.getByText(/^Limit · /)).toBeVisible();

	// Empty: blocked with a reason, nothing opens.
	await dlg.getByRole('button', { name: 'Prepare', exact: true }).click();
	await expect(dlg.getByTestId('volume-error')).toHaveText('Enter a volume in lots.');
	await expect(dlg.getByLabel('Volume (lots)')).toHaveAttribute('aria-invalid', 'true');
	await expect(modal(page)).toHaveCount(0);

	// Too many decimals and zero are blocked too.
	await dlg.getByLabel('Volume (lots)').fill('0.001');
	await dlg.getByRole('button', { name: 'Prepare', exact: true }).click();
	await expect(dlg.getByTestId('volume-error')).toHaveText('Use at most 2 decimals.');
	await dlg.getByLabel('Volume (lots)').fill('0');
	await dlg.getByRole('button', { name: 'Prepare', exact: true }).click();
	await expect(dlg.getByTestId('volume-error')).toHaveText('Volume must be greater than 0.');
	await expect(modal(page)).toHaveCount(0);

	// A valid size goes through the same confirmation modal as any other preview.
	await dlg.getByLabel('Volume (lots)').fill('0.01');
	await dlg.getByRole('button', { name: 'Prepare', exact: true }).click();
	const m = modal(page);
	await expect(m).toBeVisible();
	await expect(dlg).toBeHidden();
	await expect(m.getByTestId('preview-description')).toContainText(/Buy 0\.01 XAUUSD at market/);
	await expect(m.getByTestId('preview-description')).toContainText(/SL [\d.]+, TP [\d.]+/);
	await expect(m).toHaveAttribute('data-phase', 'review');
	await expect(m.getByTestId('countdown')).toContainText(/\d+ s left to confirm/);

	// Preparing never executed anything.
	await expect(page.getByTestId('position-row')).toHaveCount(5);
	await m.getByRole('button', { name: 'Confirm', exact: true }).click();
	await expect(m.getByTestId('result-summary')).toHaveText('Executed');
	await m.getByRole('button', { name: 'Close' }).click();
	await expect(page.getByTestId('position-row')).toHaveCount(6);
});

test('Prepare trade with a limit order places a pending order after confirmation', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'XAUUSD');
	await panel(page).getByRole('button', { name: 'Prepare trade' }).click();
	const dlg = page.getByTestId('prepare-dialog');
	await dlg.getByTestId('order-type-limit').check();
	await dlg.getByLabel('Volume (lots)').fill('0.02');
	await dlg.getByRole('button', { name: 'Prepare', exact: true }).click();

	const m = modal(page);
	await expect(m.getByTestId('preview-description')).toContainText(/Buy limit 0\.02 XAUUSD @ /);
	await m.getByRole('button', { name: 'Confirm', exact: true }).click();
	await expect(m.getByTestId('result-summary')).toHaveText('Executed');
	await m.getByRole('button', { name: 'Close' }).click();
	await expect(page.getByTestId('order-row')).toHaveCount(3);
	await expect(page.getByTestId('position-row')).toHaveCount(5);
});

test('a limit refusal is shown in the dialog and nothing opens', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'XAUUSD');
	await panel(page).getByRole('button', { name: 'Prepare trade' }).click();
	const dlg = page.getByTestId('prepare-dialog');
	await dlg.getByLabel('Volume (lots)').fill('5');
	await dlg.getByRole('button', { name: 'Prepare', exact: true }).click();
	await expect(dlg.getByTestId('prepare-error')).toContainText(/exceeds max_lot/);
	await expect(modal(page)).toHaveCount(0);
	// Esc closes it; the view is untouched.
	await page.keyboard.press('Escape');
	await expect(dlg).toBeHidden();
	await expect(page.getByTestId('market-card')).toBeVisible();
});

test('a view that has run out can no longer be prepared', async ({ page }) => {
	await openHud(page, 'viewttl=3');
	await analyze(page, 'XAUUSD');
	await expect(page.getByTestId('market-card')).toBeVisible();
	await expect(page.getByTestId('setup-expired')).toBeVisible({ timeout: 15_000 });
	await expect(panel(page).getByRole('button', { name: 'Prepare trade' })).toHaveCount(0);
	await expect(page.getByTestId('valid-until')).toContainText('expired');
});

test('the kill switch disables Prepare trade', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'XAUUSD');
	await expect(panel(page).getByRole('button', { name: 'Prepare trade' })).toBeEnabled();
	await page.getByTestId('kill-toggle').click();
	await page.getByRole('button', { name: 'Engage', exact: true }).click();
	await expect(page.getByTestId('kill-banner')).toBeVisible();
	await expect(panel(page).getByRole('button', { name: 'Prepare trade' })).toBeDisabled();
});

test('track record shows the stats and the latest views with outcome badges', async ({ page }) => {
	await openHud(page);
	await page.getByRole('button', { name: 'Market view', exact: true }).click();
	await panel(page).getByRole('tab', { name: 'Track record' }).click();

	const stats = page.getByTestId('track-stats');
	await expect(stats).toBeVisible();
	await expect(page.getByTestId('stat-setups')).toContainText('14');
	await expect(page.getByTestId('stat-tp')).toContainText('57%');
	await expect(page.getByTestId('stat-tp')).toContainText('8 of 14');
	await expect(page.getByTestId('stat-sl')).toContainText('29%');
	await expect(page.getByTestId('stat-expired')).toContainText('14%');
	await expect(page.getByTestId('stat-bias')).toContainText('60%');
	await expect(page.getByTestId('stat-bias')).toContainText('12 of 20');

	const rows = page.getByTestId('journal-row');
	await expect(rows).toHaveCount(15);
	const badges = await page.getByTestId('outcome-badge').allTextContents();
	expect(new Set(badges)).toEqual(new Set(['TP hit', 'SL hit', 'Expired', 'No setup']));
	await expect(page.getByTestId('market-disclaimer')).toBeVisible();
});

test('a new view is journaled as open', async ({ page }) => {
	await openHud(page);
	await analyze(page, 'XAUUSD');
	await expect(page.getByTestId('market-card')).toBeVisible();
	await panel(page).getByRole('tab', { name: 'Track record' }).click();
	await expect(page.getByTestId('journal-row').first()).toHaveAttribute('data-outcome', 'open');
	await expect(page.getByTestId('stat-setups')).toContainText('15');
});

test('chat tool chips for the market tools read in plain words', async ({ page }) => {
	await openHud(page);
	await say(page, "What's your view on gold?");
	const chips = page.getByRole('list', { name: 'Tools used' });
	await expect(chips).toContainText('Market view');
	await expect(page.getByRole('log')).toContainText('computed analysis');
	await say(page, 'any events today?');
	await expect(chips.last()).toContainText('Economic calendar');
	await say(page, 'gold news');
	await expect(chips.last()).toContainText('News');
	await expect(modal(page)).toHaveCount(0);
});

test('the panel is reachable by keyboard and analyzes from the watchlist chips', async ({ page }) => {
	await openHud(page);
	await page.getByRole('button', { name: 'Market view', exact: true }).click();
	const chip = panel(page).getByRole('group', { name: 'Watchlist symbols' }).getByRole('button', { name: 'EURUSD' });
	await chip.focus();
	await page.keyboard.press('Enter');
	await expect(page.getByTestId('market-card')).toHaveAttribute('data-symbol', 'EURUSD');
	await expect(chip).toHaveAttribute('aria-pressed', 'true');
});
