import { expect, test, type Page } from '@playwright/test';
import { modal } from './helpers';

// Desk mode (tasks/spec-nimbus-desk.md), in mock mode: the switch, the nine seats around the orb,
// the panels, and the one gated path for money: Sentinel, then Pilot, then the usual confirmation.

async function openDesk(page: Page) {
	await page.goto('/?mock=reset&mode=desk');
	await expect(page.getByTestId('desk-main')).toBeVisible();
}

async function ask(page: Page, text: string) {
	await page.getByRole('textbox', { name: /Message/ }).fill(text);
	await page.getByRole('button', { name: 'Send' }).click();
}

test('the mode switch opens the Desk with nine seats and every panel', async ({ page }) => {
	await page.goto('/?mock=reset&mode=trading');
	await page.getByTestId('mode-switch').getByRole('button', { name: 'Desk' }).click();
	await expect(page.getByTestId('desk-main')).toBeVisible();
	// Desk mode is always Friday: no persona choice.
	await expect(page.getByRole('group', { name: 'Persona' })).toHaveCount(0);
	await expect(page.getByTestId('brain-badge')).toContainText('claude-opus-5-5');
	for (const id of ['atlas', 'capitol', 'scout', 'athena', 'chartist', 'oracle', 'sentinel', 'pilot', 'ledger']) {
		await expect(page.getByTestId(`seat-${id}`)).toBeVisible();
	}
	await expect(page.getByTestId('session-phase')).not.toHaveText('—');
	await expect(page.getByTestId('desk-calendar')).toBeVisible();
	await expect(page.getByTestId('pulse')).toBeVisible();
	// A symbol the broker does not have shows a dash, not a number.
	await expect(page.getByTestId('pulse').getByText('NAS100').locator('..')).toContainText('—');
});

test('a desk view lights the seats, draws the chart and streams signals', async ({ page }) => {
	await openDesk(page);
	await ask(page, 'give me the desk view on gold');
	await expect(page.getByText(/Athena marks the zones on XAUUSD/)).toBeVisible();
	await expect(page.getByTestId('desk-chart')).toBeVisible();
	await expect(page.getByTestId('chart-title')).toHaveText('XAUUSD · H1');
	await expect(page.getByTestId('seat-oracle')).toHaveAttribute('data-state', 'done', { timeout: 5_000 });
	await expect(page.getByTestId('task-queue')).toContainText('oracle');
	await expect(page.getByTestId('signal-log')).toContainText('ORACLE', { ignoreCase: true, timeout: 15_000 });
});

test('Sentinel vetoes a trade without a stop and approves one with a stop', async ({ page }) => {
	await openDesk(page);
	await ask(page, 'size a long on gold');
	await expect(page.getByText(/Sentinel vetoed that/)).toBeVisible();
	const cards = page.getByTestId('plan-cards');
	await expect(cards.getByText('Vetoed')).toBeVisible();
	await expect(cards.getByTestId('send-to-pilot')).toHaveCount(0);

	await ask(page, 'size a long on gold with a stop at 2380');
	await expect(page.getByText(/Sentinel approved/)).toBeVisible();
	await expect(modal(page)).toHaveCount(0);

	// Only the button (or a spoken "buy") sends the plan to Pilot, and Pilot only makes a preview.
	await cards.getByTestId('send-to-pilot').click();
	const m = modal(page);
	await expect(m).toBeVisible();
	await expect(m.getByTestId('preview-description')).toContainText(/Buy .* XAUUSD at market/);
	await page.keyboard.press('Escape');
	await expect(m).toHaveCount(0);
	await expect(cards.getByText('Sent to Pilot')).toBeVisible();
});

test('asking to buy with a stop goes to the confirmation, never straight to the market', async ({ page }) => {
	await openDesk(page);
	await ask(page, 'buy gold with a stop at 2380 and a target at 2420');
	const m = modal(page);
	await expect(m).toBeVisible();
	await expect(m).toHaveAttribute('data-phase', 'review');
	await expect(m.getByTestId('preview-description')).toContainText(/SL 2380, TP 2420/);
	await m.getByRole('button', { name: 'Confirm', exact: true }).click();
	await expect(m).toHaveAttribute('data-phase', 'results');
	await expect(m.getByTestId('result-summary')).toHaveText('Executed');
});
