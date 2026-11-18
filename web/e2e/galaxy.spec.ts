import { expect, test } from '@playwright/test';
import { openHud, say } from './helpers';

test('the notes galaxy opens from the menu, searches, reads notes and follows wikilinks', async ({ page }) => {
	await openHud(page);
	await page.getByRole('button', { name: 'Menu', exact: true }).click();
	await page.getByRole('menuitem', { name: 'Notes galaxy' }).click();
	await expect(page).toHaveURL(/\/galaxy$/);
	await expect(page.getByTestId('note-count')).toHaveText('6 notes · 5 links');
	await expect(page.getByTestId('galaxy-canvas').locator('canvas')).toBeVisible();
	await expect(page.getByLabel('Folders')).toContainText('plans');

	await page.getByLabel('Search notes').fill('risk');
	await page.getByLabel('Matching notes').getByRole('button', { name: 'Risk per trade' }).click();
	const panel = page.getByTestId('note-panel');
	await expect(panel.getByRole('heading', { name: 'Risk per trade' })).toBeVisible();
	await expect(panel).toContainText('1% of equity');

	// A [[wikilink]] in the note opens the linked note.
	await panel.getByRole('button', { name: 'Revenge trading' }).click();
	await expect(panel.getByRole('heading', { name: 'Revenge trading' })).toBeVisible();
	await page.keyboard.press('Escape');
	await expect(panel).toBeHidden();
});

test('asking about the notes answers from them and opens the source note', async ({ page }) => {
	await page.goto('/galaxy?mock=reset');
	await expect(page.getByTestId('note-count')).toBeVisible();
	await page.getByLabel('Ask about your notes').fill('what is my rule for gold on news days?');
	await page.getByRole('button', { name: 'Ask' }).click();
	await expect(page.getByTestId('galaxy-answer')).toContainText('30 minutes');
	await expect(page.getByTestId('note-panel').getByRole('heading', { name: 'Gold news-day rule' })).toBeVisible();

	// A question the notes do not cover moves nothing.
	await page.keyboard.press('Escape');
	await page.getByLabel('Ask about your notes').fill('what do my notes say about crypto staking?');
	await page.getByRole('button', { name: 'Ask' }).click();
	await expect(page.getByTestId('galaxy-answer')).toContainText("don't cover");
	await expect(page.getByTestId('note-panel')).toBeHidden();
});

test('a HUD reply that used a note links to it in the galaxy', async ({ page }) => {
	await openHud(page);
	await say(page, 'what is my rule for gold on news days?');
	const chip = page.getByTestId('note-chip');
	await expect(chip).toHaveText(/from: Gold news-day rule/);
	await chip.click();
	await expect(page).toHaveURL(/\/galaxy\?note=1$/);
	await expect(page.getByTestId('note-panel').getByRole('heading', { name: 'Gold news-day rule' })).toBeVisible();
});

test.describe('at 375 px', () => {
	test.use({ viewport: { width: 375, height: 740 } });
	test('the galaxy fits the screen and the note opens as a bottom sheet', async ({ page }) => {
		await page.goto('/galaxy?mock=reset&note=2');
		const panel = page.getByTestId('note-panel');
		await expect(panel.getByRole('heading', { name: 'Risk per trade' })).toBeVisible();
		const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
		expect(overflow).toBeLessThanOrEqual(0);
		const box = await panel.boundingBox();
		expect(box && box.y).toBeGreaterThan(200);
	});
});
