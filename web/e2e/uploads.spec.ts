import { expect, test, type Page } from '@playwright/test';

// Uploads (tasks/spec-model-uploads.md), in mock mode: 3D files open in the sandbox, pictures
// become a relief or a Claude build, bad files fail safely, and drag and drop works.

/** A valid one-triangle glTF binary. */
function glb(): Buffer {
	const json = JSON.stringify({
		asset: { version: '2.0' },
		scene: 0,
		scenes: [{ nodes: [0] }],
		nodes: [{ mesh: 0, name: 'Wing' }],
		meshes: [{ name: 'Wing', primitives: [{ attributes: { POSITION: 0 } }] }],
		accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', max: [1, 1, 0], min: [0, 0, 0] }],
		bufferViews: [{ buffer: 0, byteLength: 36 }],
		buffers: [{ byteLength: 36 }]
	});
	const jsonChunk = Buffer.from(json.padEnd(Math.ceil(json.length / 4) * 4, ' '));
	const bin = Buffer.alloc(36);
	[0, 0, 0, 1, 0, 0, 0, 1, 0].forEach((v, i) => bin.writeFloatLE(v, i * 4));
	const header = Buffer.alloc(12);
	const total = 12 + 8 + jsonChunk.length + 8 + bin.length;
	header.write('glTF', 0, 'ascii');
	header.writeUInt32LE(2, 4);
	header.writeUInt32LE(total, 8);
	const chunk = (data: Buffer, type: number) => {
		const h = Buffer.alloc(8);
		h.writeUInt32LE(data.length, 0);
		h.writeUInt32LE(type, 4);
		return Buffer.concat([h, data]);
	};
	return Buffer.concat([header, chunk(jsonChunk, 0x4e4f534a), chunk(bin, 0x004e4942)]);
}

const OBJ = Buffer.from('# a tetrahedron\no Pyramid\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\nf 1 2 4\nf 1 3 4\nf 2 3 4\n');
const STL = Buffer.from('solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n');
const PLY = Buffer.from(
	'ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n'
);
const PNG = Buffer.from(
	'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
	'base64'
);

async function openAssistant(page: Page, flags = 'reset') {
	await page.goto(`/?mock=${flags}&mode=assistant`);
	await expect(page.getByTestId('assistant-main')).toBeVisible();
}

async function pick(page: Page, name: string, buffer: Buffer, mimeType = 'application/octet-stream') {
	await page.getByTestId('upload-input').setInputFiles({ name, mimeType, buffer });
}

const stage = (page: Page) => page.getByTestId('model-stage');

for (const [name, buffer, title] of [
	['jet_wing.glb', glb(), 'jet wing'],
	['pyramid.obj', OBJ, 'pyramid'],
	['part.stl', STL, 'part'],
	['scan.ply', PLY, 'scan']
] as const) {
	test(`a ${name.split('.')[1]} file opens in 3D for free`, async ({ page }) => {
		await openAssistant(page);
		await pick(page, name, buffer);
		await expect(stage(page).getByRole('heading')).toContainText(title);
		await expect(stage(page).getByText(/tris/)).toBeVisible({ timeout: 15_000 });
		await expect(page.getByTestId('model-chip')).toContainText(title);
	});
}

test('a picture can become a free 3D relief', async ({ page }) => {
	await openAssistant(page);
	await pick(page, 'holiday.png', PNG, 'image/png');
	const sheet = page.getByTestId('image-sheet');
	await expect(sheet).toBeVisible();
	await sheet.getByTestId('image-relief').click();
	await expect(sheet).toHaveCount(0);
	await expect(stage(page).getByRole('heading')).toContainText('holiday');
	await expect(stage(page).getByText(/tris/)).toBeVisible({ timeout: 15_000 });
});

test('Claude builds a model from a picture with a note', async ({ page }) => {
	await openAssistant(page);
	await pick(page, 'plan.png', PNG, 'image/png');
	const sheet = page.getByTestId('image-sheet');
	await sheet.getByLabel('For Claude (optional)').fill('my 1 BHK floor plan, walls 3 m');
	await sheet.getByTestId('image-build').click();
	await expect(page.getByTestId('upload-busy')).toBeVisible();
	await expect(stage(page).getByRole('heading')).toContainText('1 BHK flat', { timeout: 15_000 });
	await expect(stage(page).getByText(/tris/)).toBeVisible({ timeout: 15_000 });
	await expect(page.getByText('[picture: plan.png] my 1 BHK floor plan, walls 3 m')).toBeVisible();
});

test('without Claude, the picture sheet offers only the free relief', async ({ page }) => {
	await openAssistant(page, 'noclaude');
	await pick(page, 'plan.png', PNG, 'image/png');
	await expect(page.getByTestId('image-build')).toBeDisabled();
	await expect(page.getByTestId('image-relief')).toBeEnabled();
});

test('a broken file fails inside the sandbox and the app stays usable', async ({ page }) => {
	await openAssistant(page);
	await pick(page, 'broken.glb', Buffer.from('glTF this is not really a model'));
	await expect(stage(page).getByRole('alert')).toContainText('could not be shown', { timeout: 15_000 });
	await page.getByTestId('mode-switch').getByRole('button', { name: 'Trading' }).click();
	await expect(page.locator('#panel-watchlist')).toBeVisible();
});

test('an unsupported file is refused with a plain note', async ({ page }) => {
	await openAssistant(page);
	await pick(page, 'setup.exe', Buffer.from('MZ'));
	await expect(page.getByTestId('note').last()).toContainText('cannot be shown in 3D');
	await expect(stage(page)).toHaveCount(0);
});

/** Drop `name` (with `text` inside) on the page the way a browser does: enter, then drop. */
async function dropFile(page: Page, name: string, text: string) {
	const drop = await page.evaluateHandle(
		([n, t]) => {
			const dt = new DataTransfer();
			dt.items.add(new File([t], n, { type: '' }));
			return dt;
		},
		[name, text]
	);
	await page.locator('body').dispatchEvent('dragenter', { dataTransfer: drop });
	const overlay = page.getByTestId('drop-overlay');
	await expect(overlay).toBeVisible();
	await overlay.dispatchEvent('dragover', { dataTransfer: drop });
	await overlay.dispatchEvent('drop', { dataTransfer: drop });
	await expect(overlay).toHaveCount(0);
}

test('a file dropped on the page opens, even over a model already on screen', async ({ page }) => {
	await openAssistant(page);
	await dropFile(page, 'dropped.obj', OBJ.toString());
	await expect(stage(page).getByRole('heading')).toContainText('dropped');
	await expect(stage(page).getByText(/tris/)).toBeVisible({ timeout: 15_000 });
	// A second file dropped while the first is on screen replaces it.
	await dropFile(page, 'second.obj', OBJ.toString());
	await expect(stage(page).getByRole('heading')).toContainText('second');
	await expect(stage(page).getByText(/tris/)).toBeVisible({ timeout: 15_000 });
});

/** A one-triangle GLB with a PNG texture embedded in its binary chunk. */
function texturedGlb(): Buffer {
	const pos = Buffer.alloc(36);
	[0, 0, 0, 1, 0, 0, 0, 1, 0].forEach((v, i) => pos.writeFloatLE(v, i * 4));
	const uv = Buffer.alloc(24);
	[0, 0, 1, 0, 0, 1].forEach((v, i) => uv.writeFloatLE(v, i * 4));
	const png = Buffer.concat([PNG, Buffer.alloc((4 - (PNG.length % 4)) % 4)]);
	const bin = Buffer.concat([pos, uv, png]);
	const json = JSON.stringify({
		asset: { version: '2.0' },
		scene: 0,
		scenes: [{ nodes: [0] }],
		nodes: [{ mesh: 0, name: 'Painted' }],
		meshes: [{ name: 'Painted', primitives: [{ attributes: { POSITION: 0, TEXCOORD_0: 1 }, material: 0 }] }],
		materials: [{ pbrMetallicRoughness: { baseColorTexture: { index: 0 } } }],
		textures: [{ source: 0 }],
		images: [{ bufferView: 2, mimeType: 'image/png' }],
		accessors: [
			{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', max: [1, 1, 0], min: [0, 0, 0] },
			{ bufferView: 1, componentType: 5126, count: 3, type: 'VEC2' }
		],
		bufferViews: [
			{ buffer: 0, byteOffset: 0, byteLength: 36 },
			{ buffer: 0, byteOffset: 36, byteLength: 24 },
			{ buffer: 0, byteOffset: 60, byteLength: PNG.length }
		],
		buffers: [{ byteLength: bin.length }]
	});
	const jsonChunk = Buffer.from(json.padEnd(Math.ceil(json.length / 4) * 4, ' '));
	const chunk = (data: Buffer, type: number) => {
		const h = Buffer.alloc(8);
		h.writeUInt32LE(data.length, 0);
		h.writeUInt32LE(type, 4);
		return Buffer.concat([h, data]);
	};
	const body = Buffer.concat([chunk(jsonChunk, 0x4e4f534a), chunk(bin, 0x004e4942)]);
	const header = Buffer.alloc(12);
	header.write('glTF', 0, 'ascii');
	header.writeUInt32LE(2, 4);
	header.writeUInt32LE(12 + body.length, 8);
	return Buffer.concat([header, body]);
}

test('inside the sandbox, embedded textures load and the network stays out of reach', async ({ page }) => {
	await openAssistant(page);
	// Code run on the loaded model, as Claude's change would be: it reports what it could reach.
	const probe = `
const r = [];
let map = false;
helpers.model.traverse((o) => { if (o.material && o.material.map) map = true; });
r.push('map=' + map);
const get = (u) => { try { const x = new XMLHttpRequest(); x.open('GET', u, false); x.send(); return 'open'; } catch (e) { return 'blocked'; } };
r.push('http=' + get('http://127.0.0.1:8787/api/status'));
r.push('site=' + get('http://127.0.0.1:5173/'));
r.push('ownblob=' + get(URL.createObjectURL(new Blob(['x']))));
r.push('appblob=' + get(APP_BLOB));
let ws = 'blocked';
try { new WebSocket('ws://127.0.0.1:8787/api/ws'); ws = 'open'; } catch (e) {}
r.push('ws=' + ws);
throw new Error(r.join(','));`;
	const result = await page.evaluate(
		async ({ bytes, probe }) => {
			const appBlob = URL.createObjectURL(new Blob(['secret from the app']));
			const f = document.createElement('iframe');
			f.sandbox.add('allow-scripts');
			f.src = '/sandbox/model.html';
			document.body.appendChild(f);
			return await new Promise<string>((resolve) => {
				setTimeout(() => resolve('timeout'), 20_000);
				window.addEventListener('message', (e) => {
					if (e.source !== f.contentWindow) return;
					if (e.data?.type === 'loaded') {
						const buf = new Uint8Array(bytes).buffer;
						const overlay = probe.replace('APP_BLOB', JSON.stringify(appBlob));
						f.contentWindow!.postMessage({ type: 'load', format: 'glb', bytes: buf, overlay }, '*', [buf]);
					}
					if (e.data?.type === 'error') resolve(e.data.message);
					if (e.data?.type === 'ready') resolve('ready without the probe');
				});
			});
		},
		{ bytes: [...texturedGlb()], probe }
	);
	expect(result).toContain('map=true'); // the embedded texture was read (blob: allowed)
	expect(result).toContain('http=blocked');
	expect(result).toContain('site=blocked');
	expect(result).toContain('ownblob=open');
	expect(result).toContain('appblob=blocked'); // the app's own blob URLs are another origin
});

test.describe('at 375 px', () => {
	test.use({ viewport: { width: 375, height: 812 }, hasTouch: true });

	test('the picture sheet fits a phone', async ({ page }) => {
		await openAssistant(page);
		await pick(page, 'plan.png', PNG, 'image/png');
		await expect(page.getByTestId('image-sheet')).toBeVisible();
		const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
		expect(overflow).toBeLessThanOrEqual(0);
		const box = await page.getByTestId('image-sheet').boundingBox();
		expect(box!.x + box!.width).toBeLessThanOrEqual(375);
	});
});
