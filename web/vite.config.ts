import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig, type Plugin } from 'vite';
import { buildSandbox } from './scripts/build-sandbox.mjs';

// The Rust server (jarvis-api) listens here; see docs/api.md.
const API_TARGET = 'http://127.0.0.1:8787';

const SANDBOX_CSP =
	"default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; style-src 'unsafe-inline'; " +
	"img-src data: blob:; connect-src blob: data:; font-src 'none'; media-src 'none'; worker-src blob:; " +
	"frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; " +
	"frame-ancestors 'self'; sandbox allow-scripts";

/**
 * Builds static/sandbox/model.html, the page that runs Claude-built 3D models in a sandbox
 * (spec-assistant-mode), before the dev server, a build or the end-to-end server starts. It only
 * rebuilds when its sources changed. Skipped under Vitest.
 */
function sandboxPage(): Plugin {
	// The same policy the Rust server sends (crates/api/src/security.rs, SANDBOX_CSP): a meta tag
	// cannot carry `sandbox` or `frame-ancestors`, so the dev and preview servers send it too.
	const headers = (req: { url?: string }, res: { setHeader(k: string, v: string): void }, next: () => void) => {
		if (req.url?.split('?')[0] === '/sandbox/model.html') {
			res.setHeader('Content-Security-Policy', SANDBOX_CSP);
			res.setHeader('X-Frame-Options', 'SAMEORIGIN');
		}
		next();
	};
	return {
		name: 'arc-sandbox-page',
		async buildStart() {
			if (process.env.VITEST) return;
			if (await buildSandbox()) this.info('built static/sandbox/model.html');
		},
		configureServer(server) {
			server.middlewares.use(headers);
		},
		configurePreviewServer(server) {
			server.middlewares.use(headers);
		}
	};
}

export default defineConfig({
	build: {
		// three.js is lazy-loaded (dynamic import in Orb.svelte), so its ~550 kB chunk is expected.
		chunkSizeWarningLimit: 700,
		// Never inline fonts as data: URLs. The server's CSP allows fonts from 'self' only,
		// so an inlined font would be blocked; emit them as files instead.
		assetsInlineLimit: (file) => (/\.(woff2?|ttf|otf|eot)$/i.test(file) ? false : undefined)
	},
	// Audio worklets are bundled as ES modules and loaded by URL (see src/lib/voice/*.worklet.js).
	worker: { format: 'es' },
	// The orb and the notes galaxy load three.js lazily. Pre-bundle it (and the orbit controls) at
	// dev-server start, so the first visit does not trigger a dependency re-optimise and a page
	// reload in the middle of a session or an end-to-end test.
	optimizeDeps: { include: ['three', 'three/addons/controls/OrbitControls.js'] },
	plugins: [
		sandboxPage(),
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// Single-page app: the Rust server serves build/ and falls back to index.html.
			adapter: adapter({ fallback: 'index.html' })
		})
	],
	server: {
		proxy: { '/api': { target: API_TARGET, ws: true } }
	},
	preview: {
		proxy: { '/api': { target: API_TARGET, ws: true } }
	}
});
