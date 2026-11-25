import { expect, test } from '@playwright/test';

// Alpaca as a second venue (tasks/spec-alpaca.md), in mock mode: `?mock=alpaca` turns it on.

test('Alpaca rows carry a venue tag and the account panel shows the paper account', async ({ page }) => {
	await page.goto('/?mock=alpaca&mode=trading');
	const row = page.getByTestId('position-row').filter({ hasText: 'NVDA' });
	await expect(row).toBeVisible();
	await expect(row.getByTestId('venue-tag')).toHaveText('Alpaca');
	const account = page.getByTestId('alpaca-account');
	await expect(account).toContainText(/Alpaca paper/i);
	await expect(account).toContainText('100,002.75');
});

test('without Alpaca nothing changes: no venue tags, no Alpaca account', async ({ page }) => {
	await page.goto('/?mock=reset&mode=trading');
	await expect(page.getByTestId('position-row')).toHaveCount(5);
	await expect(page.getByTestId('venue-tag')).toHaveCount(0);
	await expect(page.getByTestId('alpaca-account')).toHaveCount(0);
});
