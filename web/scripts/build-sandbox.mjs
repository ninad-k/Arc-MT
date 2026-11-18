// Builds the 3D model sandbox page, static/sandbox/model.html (spec-assistant-mode, ADR-0022):
// the page script (src/lib/model3d/sandbox-runtime.ts) inline, with the model worker
// (sandbox-worker.ts, three.js bundled in) inlined into it as a string it starts from a blob.
// A Content-Security-Policy meta tag blocks all network access; the server sends the same policy
// as a header (crates/api/src/security.rs, SANDBOX_CSP).
//
// Generated, not committed (see .gitignore). The Vite config runs this before dev, build and the
// end-to-end server; it rebuilds only when the runtime or this script is newer than the page.
// Standalone: node scripts/build-sandbox.mjs [--force]

import { existsSync, mkdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const entry = resolve(root, 'src/lib/model3d/sandbox-runtime.ts');
const workerEntry = resolve(root, 'src/lib/model3d/sandbox-worker.ts');
const protocol = resolve(root, 'src/lib/model3d/protocol.ts');
const out = resolve(root, 'static/sandbox/model.html');

const CSP =
	"default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; style-src 'unsafe-inline'; " +
	"img-src data: blob:; connect-src blob: data:; font-src 'none'; media-src 'none'; worker-src blob:; " +
	"frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";

function stale() {
	if (!existsSync(out)) return true;
	const built = statSync(out).mtimeMs;
	return [entry, workerEntry, protocol, fileURLToPath(import.meta.url)].some((f) => statSync(f).mtimeMs > built);
}

/** Build the page when it is missing or older than its sources. Returns true when it was built. */
export async function buildSandbox({ force = false } = {}) {
	if (!force && !stale()) return false;
	const { build } = await import('vite');
	/**
	 * One self-contained script for `file`, with `define` replacements.
	 * @param {string} file
	 * @param {string} name
	 * @param {Record<string, string>} [define]
	 */
	const bundle = async (file, name, define = {}) => {
		const result = await build({
			configFile: false,
			logLevel: 'warn',
			root,
			define,
			build: {
				write: false,
				minify: true,
				sourcemap: false,
				lib: { entry: file, formats: ['iife'], name, fileName: () => `${name}.js` }
			}
		});
		/** @type {any[]} */
		const results = Array.isArray(result) ? result : [result];
		const chunk = results.flatMap((r) => r.output ?? []).find((o) => o.type === 'chunk');
		if (!chunk) throw new Error(`the sandbox build of ${file} produced no code`);
		return chunk.code;
	};
	const workerCode = await bundle(workerEntry, 'ArcSandboxWorker');
	const pageCode = await bundle(entry, 'ArcSandbox', { __SANDBOX_WORKER__: JSON.stringify(workerCode) });
	// An inline script ends at the first "</script": make sure the bundle never contains one.
	const code = pageCode.replace(/<\/script/gi, '<\\/script');
	const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${CSP}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>3D model</title>
<style>html,body{margin:0;height:100%;overflow:hidden;background:#05070d}canvas{display:block;width:100%;height:100%;touch-action:none}</style>
</head>
<body>
<script>${code}</script>
</body>
</html>
`;
	mkdirSync(dirname(out), { recursive: true });
	writeFileSync(out, html);
	return true;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
	const built = await buildSandbox({ force: process.argv.includes('--force') });
	console.log(built ? `built ${out}` : `${out} is up to date`);
}
