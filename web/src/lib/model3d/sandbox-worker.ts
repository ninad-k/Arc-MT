/**
 * The 3D model worker (spec-assistant-mode, ADR-0022). The model's code runs here, never on a
 * page's thread, so a model that never stops can be terminated by the sandbox page and cannot
 * freeze the app. It draws to an OffscreenCanvas handed over by the page, and the page forwards
 * dragging and zooming as messages.
 *
 * It is created from a blob by the sandbox page and inherits that page's policy: no network
 * (only local `blob:` and `data:` reads, which the glTF loader uses for embedded textures), and
 * an opaque origin with no storage.
 *
 * Uploaded files (spec-model-uploads) are parsed here too, never in the app or on the server:
 * glTF/GLB, OBJ, STL and PLY with the three.js loaders, and pictures as a raised relief.
 */
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { OBJLoader } from 'three/addons/loaders/OBJLoader.js';
import { STLLoader } from 'three/addons/loaders/STLLoader.js';
import { PLYLoader } from 'three/addons/loaders/PLYLoader.js';

type In =
	| { type: 'init'; canvas: OffscreenCanvas; width: number; height: number; dpr: number }
	| { type: 'run'; code: string }
	| { type: 'load'; format: string; bytes: ArrayBuffer; overlay?: string | null }
	| { type: 'relief'; bytes: ArrayBuffer; overlay?: string | null }
	| { type: 'resize'; width: number; height: number }
	| { type: 'orbit'; dx: number; dy: number }
	| { type: 'zoom'; factor: number }
	| { type: 'reset-view' }
	| { type: 'pause'; paused: boolean };

const ctx = self as unknown as {
	postMessage(message: unknown): void;
	onmessage: ((event: MessageEvent<In>) => void) | null;
	requestAnimationFrame?: (cb: (t: number) => void) => number;
};
const post = (m: unknown) => ctx.postMessage(m);

let renderer: THREE.WebGLRenderer | null = null;
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(45, 1, 0.05, 2000);
let update: ((t: number, dt: number) => void) | null = null;
let paused = false;
let autoRotate = true;

// A simple orbit around `target`: the page sends drag and zoom deltas.
const target = new THREE.Vector3();
const orbit = { radius: 12, theta: 0.6, phi: 1.1 };

function placeCamera(): void {
	orbit.phi = Math.min(Math.max(orbit.phi, 0.05), Math.PI - 0.05);
	camera.position.setFromSphericalCoords(orbit.radius, orbit.phi, orbit.theta).add(target);
	camera.lookAt(target);
}

function label(text: string, opts: { color?: string; size?: number } = {}): THREE.Sprite {
	const font = 48;
	const probe = new OffscreenCanvas(8, 8).getContext('2d')!;
	probe.font = `600 ${font}px sans-serif`;
	const canvas = new OffscreenCanvas(Math.ceil(probe.measureText(text).width) + 24, font + 24);
	const c = canvas.getContext('2d')!;
	c.fillStyle = 'rgba(2, 6, 23, 0.6)';
	c.fillRect(0, 0, canvas.width, canvas.height);
	c.font = `600 ${font}px sans-serif`;
	c.fillStyle = opts.color ?? '#e2e8f0';
	c.textBaseline = 'middle';
	c.fillText(text, 12, canvas.height / 2);
	const texture = new THREE.CanvasTexture(canvas as unknown as HTMLCanvasElement);
	texture.colorSpace = THREE.SRGBColorSpace;
	const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: texture, depthTest: false }));
	const size = opts.size ?? 1;
	sprite.scale.set((size * canvas.width) / canvas.height, size, 1);
	return sprite;
}

function clearScene(): void {
	scene.traverse((o) => {
		const mesh = o as THREE.Mesh;
		mesh.geometry?.dispose?.();
		const m = mesh.material as THREE.Material | THREE.Material[] | undefined;
		(Array.isArray(m) ? m : m ? [m] : []).forEach((mat) => mat.dispose());
	});
	scene.clear();
	scene.background = new THREE.Color('#05070d');
	scene.add(new THREE.HemisphereLight('#dbeafe', '#0b1020', 1.1));
	const key = new THREE.DirectionalLight('#ffffff', 2.2);
	key.position.set(6, 10, 8);
	scene.add(key);
	const rim = new THREE.DirectionalLight('#7dd3fc', 0.8);
	rim.position.set(-8, 4, -6);
	scene.add(rim);
}

/** Fit the camera to whatever was built. */
function frame(): void {
	const box = new THREE.Box3().setFromObject(scene);
	if (box.isEmpty()) {
		target.set(0, 0, 0);
		orbit.radius = 12;
	} else {
		const sphere = box.getBoundingSphere(new THREE.Sphere());
		target.copy(sphere.center);
		orbit.radius = (Math.max(sphere.radius, 0.5) / Math.sin(THREE.MathUtils.degToRad(camera.fov / 2))) * 1.15;
		camera.near = Math.max(orbit.radius / 1000, 0.01);
		camera.far = orbit.radius * 20;
		camera.updateProjectionMatrix();
	}
	orbit.theta = 0.6;
	orbit.phi = 1.1;
	autoRotate = true;
	placeCamera();
}

function triangles(): number {
	let n = 0;
	scene.traverse((o) => {
		const g = (o as THREE.Mesh).geometry as THREE.BufferGeometry | undefined;
		if (!g) return;
		const count = g.index ? g.index.count : (g.attributes.position?.count ?? 0);
		const instances = (o as THREE.InstancedMesh).isInstancedMesh ? (o as THREE.InstancedMesh).count : 1;
		n += Math.floor(count / 3) * instances;
	});
	return n;
}

/** Install what Claude's code returned: an update function, or an object with one. */
function takeUpdate(result: unknown): void {
	if (typeof result === 'function') update = result as typeof update;
	else if (result && typeof (result as { update?: unknown }).update === 'function') {
		const r = result as { update: (t: number, dt: number) => void };
		update = r.update.bind(r);
	}
}

const DEFAULT_MATERIAL = () => new THREE.MeshStandardMaterial({ color: '#a8b3c2', metalness: 0.25, roughness: 0.55 });

/** Parse an uploaded 3D file into an object. */
async function parseFile(format: string, bytes: ArrayBuffer): Promise<THREE.Object3D> {
	switch (format) {
		case 'glb':
		case 'gltf': {
			const data = format === 'gltf' ? new TextDecoder().decode(bytes) : bytes;
			const gltf = await new GLTFLoader().parseAsync(data, '');
			return gltf.scene;
		}
		case 'obj': {
			const group = new OBJLoader().parse(new TextDecoder().decode(bytes));
			// OBJ files without a material file get a plain, lit material.
			group.traverse((o) => {
				const mesh = o as THREE.Mesh;
				if (mesh.isMesh && (mesh.material as THREE.Material).type === 'MeshPhongMaterial') mesh.material = DEFAULT_MATERIAL();
			});
			return group;
		}
		case 'stl':
		case 'ply': {
			const geometry = format === 'stl' ? new STLLoader().parse(bytes) : new PLYLoader().parse(bytes);
			const colours = geometry.hasAttribute('color');
			const hasFaces = format === 'stl' || geometry.index !== null;
			if (!hasFaces) {
				// A PLY point cloud.
				// Points a few pixels wide whatever the scan's units.
				return new THREE.Points(
					geometry,
					new THREE.PointsMaterial({ size: 2, sizeAttenuation: false, vertexColors: colours, color: colours ? '#ffffff' : '#a8b3c2' })
				);
			}
			if (!geometry.hasAttribute('normal')) geometry.computeVertexNormals();
			const material = DEFAULT_MATERIAL();
			if (colours) {
				material.vertexColors = true;
				material.color.set('#ffffff');
			}
			return new THREE.Mesh(geometry, material);
		}
		default:
			throw new Error(`${format} files are not supported`);
	}
}

/** A picture as a raised relief panel: brighter stands higher, with the picture on top. */
async function reliefOf(bytes: ArrayBuffer): Promise<THREE.Object3D> {
	const bitmap = await createImageBitmap(new Blob([bytes]));
	const aspect = bitmap.height / Math.max(bitmap.width, 1);
	const gx = Math.max(2, Math.min(256, bitmap.width));
	const gy = Math.max(2, Math.min(256, Math.round(gx * aspect)));
	const sample = new OffscreenCanvas(gx, gy).getContext('2d', { willReadFrequently: true })!;
	sample.drawImage(bitmap, 0, 0, gx, gy);
	const pixels = sample.getImageData(0, 0, gx, gy).data;

	const width = 10;
	const height = width * aspect;
	const geometry = new THREE.PlaneGeometry(width, height, gx - 1, gy - 1);
	const pos = geometry.attributes.position;
	const depth = 0.9;
	for (let i = 0; i < pos.count; i++) {
		const p = i * 4;
		const lum = (0.2126 * pixels[p] + 0.7152 * pixels[p + 1] + 0.0722 * pixels[p + 2]) / 255;
		pos.setZ(i, lum * depth);
	}
	geometry.computeVertexNormals();

	const texSize = Math.min(1024, Math.max(bitmap.width, bitmap.height));
	const texCanvas = new OffscreenCanvas(Math.round(texSize / Math.max(aspect, 1)), Math.round(texSize * Math.min(aspect, 1)));
	texCanvas.getContext('2d')!.drawImage(bitmap, 0, 0, texCanvas.width, texCanvas.height);
	bitmap.close();
	const texture = new THREE.CanvasTexture(texCanvas as unknown as HTMLCanvasElement);
	texture.colorSpace = THREE.SRGBColorSpace;

	const relief = new THREE.Group();
	relief.name = 'relief';
	const surface = new THREE.Mesh(geometry, new THREE.MeshStandardMaterial({ map: texture, roughness: 0.75 }));
	surface.name = 'surface';
	const back = new THREE.Mesh(
		new THREE.BoxGeometry(width * 1.04, height * 1.04, 0.3),
		new THREE.MeshStandardMaterial({ color: '#1e293b', roughness: 0.9 })
	);
	back.name = 'frame';
	back.position.z = -0.16;
	relief.add(back, surface);
	// Stand it up, leaning back a little.
	relief.rotation.x = -0.25;
	return relief;
}

/** Part names and overall size of a loaded object, for a later change by Claude. */
function infoOf(object: THREE.Object3D): { parts: string[]; size: [number, number, number] } {
	const parts = new Set<string>();
	object.traverse((o) => {
		if (((o as THREE.Mesh).isMesh || (o as THREE.Points).isPoints) && o.name) parts.add(o.name.slice(0, 80));
	});
	const size = new THREE.Box3().setFromObject(object).getSize(new THREE.Vector3());
	const round = (n: number) => Math.round(n * 1000) / 1000;
	return { parts: [...parts].slice(0, 200), size: [round(size.x), round(size.y), round(size.z)] };
}

/** Show an uploaded file or relief, then run Claude's change to it, if any. */
async function show(make: () => Promise<THREE.Object3D>, overlay: string | null | undefined, report: boolean): Promise<void> {
	update = null;
	clearScene();
	try {
		const model = await make();
		model.name ||= 'model';
		scene.add(model);
		const info = report ? infoOf(model) : undefined;
		if (overlay) {
			const change = new Function('THREE', 'scene', 'camera', 'helpers', `"use strict";\n${overlay}`);
			takeUpdate(change(THREE, scene, camera, { label, model }));
		}
		frame();
		post({ type: 'ready', triangles: triangles(), ...(info ? { info } : {}) });
	} catch (e) {
		post({ type: 'error', message: e instanceof Error ? e.message : String(e) });
	}
}

function run(code: string): void {
	update = null;
	clearScene();
	try {
		// The model's code: a function body called with three.js, the scene, the camera and helpers.
		const build = new Function('THREE', 'scene', 'camera', 'helpers', `"use strict";\n${code}`);
		takeUpdate(build(THREE, scene, camera, { label }));
		frame();
		post({ type: 'ready', triangles: triangles() });
	} catch (e) {
		post({ type: 'error', message: e instanceof Error ? e.message : String(e) });
	}
}

let last = performance.now();
let elapsed = 0;
let frames = 0;
let lastTick = last;
function loop(): void {
	const now = performance.now();
	const dt = Math.min((now - last) / 1000, 0.1);
	last = now;
	if (!paused && renderer) {
		elapsed += dt;
		if (update) {
			try {
				update(elapsed, dt);
			} catch (e) {
				update = null;
				post({ type: 'error', message: `animation stopped: ${e instanceof Error ? e.message : String(e)}` });
			}
		}
		if (autoRotate) orbit.theta += dt * 0.15;
		placeCamera();
		renderer.render(scene, camera);
	}
	frames++;
	if (now - lastTick >= 1000) {
		post({ type: 'tick', fps: Math.round((frames * 1000) / (now - lastTick)) });
		frames = 0;
		lastTick = now;
	}
	if (ctx.requestAnimationFrame) ctx.requestAnimationFrame(loop);
	else setTimeout(loop, 16);
}

function resize(width: number, height: number): void {
	renderer?.setSize(Math.max(width, 1), Math.max(height, 1), false);
	camera.aspect = Math.max(width, 1) / Math.max(height, 1);
	camera.updateProjectionMatrix();
}

ctx.onmessage = (event) => {
	const m = event.data;
	switch (m?.type) {
		case 'init':
			renderer = new THREE.WebGLRenderer({ canvas: m.canvas, antialias: true });
			renderer.setPixelRatio(Math.min(m.dpr || 1, 2));
			renderer.toneMapping = THREE.ACESFilmicToneMapping;
			resize(m.width, m.height);
			clearScene();
			frame();
			loop();
			post({ type: 'loaded' });
			break;
		case 'run':
			run(m.code);
			break;
		case 'load':
			void show(() => parseFile(m.format, m.bytes), m.overlay, true);
			break;
		case 'relief':
			void show(() => reliefOf(m.bytes), m.overlay, true);
			break;
		case 'resize':
			resize(m.width, m.height);
			break;
		case 'orbit':
			autoRotate = false;
			orbit.theta -= m.dx * 0.008;
			orbit.phi -= m.dy * 0.008;
			break;
		case 'zoom':
			orbit.radius = Math.min(Math.max(orbit.radius * m.factor, 0.5), 2000);
			break;
		case 'reset-view':
			frame();
			break;
		case 'pause':
			paused = m.paused === true;
			break;
	}
};
