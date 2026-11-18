/**
 * The notes galaxy (ADR-0021): one glowing star per note on black space with a starfield, lines
 * between linked notes, slow idle rotation, and a camera that flies to a note or a cluster.
 * Positions come from ./layout.ts; this file only draws and handles the pointer.
 */
import {
	AdditiveBlending,
	BufferAttribute,
	BufferGeometry,
	CanvasTexture,
	Color,
	InstancedMesh,
	LineBasicMaterial,
	LineSegments,
	Matrix4,
	MeshBasicMaterial,
	PerspectiveCamera,
	Points,
	PointsMaterial,
	Quaternion,
	Raycaster,
	Scene,
	SphereGeometry,
	Vector2,
	Vector3,
	WebGLRenderer
} from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

export interface GalaxyNode {
	id: number;
	title: string;
	group: string;
	degree: number;
}

export interface GalaxyLink {
	source: number;
	target: number;
}

export interface GalaxyOptions {
	nodes: GalaxyNode[];
	links: GalaxyLink[];
	/** `[x, y, z]` per node, in the order of `nodes`. */
	positions: Float32Array;
	/** Colour per group name (CSS colour). */
	colors: Map<string, string>;
	reducedMotion: boolean;
	onSelect: (id: number | null) => void;
	onHover?: (id: number | null) => void;
	onContextLost?: () => void;
}

const FLY_MS = 900;
const IDLE_RESUME_MS = 8000;

/** A soft round sprite for the glow and the starfield. */
function glowTexture(): CanvasTexture {
	const c = document.createElement('canvas');
	c.width = c.height = 64;
	const g = c.getContext('2d')!;
	const grad = g.createRadialGradient(32, 32, 0, 32, 32, 32);
	grad.addColorStop(0, 'rgba(255,255,255,1)');
	grad.addColorStop(0.25, 'rgba(255,255,255,0.55)');
	grad.addColorStop(1, 'rgba(255,255,255,0)');
	g.fillStyle = grad;
	g.fillRect(0, 0, 64, 64);
	return new CanvasTexture(c);
}

const ease = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

export class GalaxyScene {
	private readonly renderer: WebGLRenderer;
	private readonly scene = new Scene();
	private readonly camera: PerspectiveCamera;
	private readonly controls: OrbitControls;
	private readonly stars: InstancedMesh;
	private readonly glow: Points;
	private readonly lines: LineSegments;
	private readonly hot: LineSegments;
	private readonly label: HTMLDivElement;
	private readonly raycaster = new Raycaster();
	private readonly pointer = new Vector2();
	private readonly baseColors: Color[];
	private readonly sizes: number[];
	private readonly index: Map<number, number>;
	private readonly disposables: { dispose(): void }[] = [];
	private readonly ro: ResizeObserver;
	private raf = 0;
	private selected: number | null = null;
	private hovered: number | null = null;
	private highlight: Set<number> | null = null;
	private fly: { from: Vector3; to: Vector3; fromT: Vector3; toT: Vector3; start: number } | null = null;
	private lastInteraction = 0;
	private readonly onLost = (e: Event) => {
		e.preventDefault();
		this.opts.onContextLost?.();
	};
	private down: { x: number; y: number } | null = null;
	/** Pixels on the right covered by the note panel; the view centre moves left of it. */
	private insetRight = 0;

	constructor(
		private readonly container: HTMLElement,
		private readonly opts: GalaxyOptions
	) {
		this.renderer = new WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' });
		this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
		this.renderer.setClearColor(0x02040a, 1);
		this.renderer.domElement.style.display = 'block';
		this.renderer.domElement.style.touchAction = 'none';
		container.appendChild(this.renderer.domElement);
		this.renderer.domElement.addEventListener('webglcontextlost', this.onLost);

		this.camera = new PerspectiveCamera(55, 1, 0.1, 4000);
		this.index = new Map(opts.nodes.map((n, i) => [n.id, i]));
		const radius = this.boundingRadius();
		this.camera.position.set(0, radius * 0.35, radius * 2.4 + 20);

		this.controls = new OrbitControls(this.camera, this.renderer.domElement);
		this.controls.enableDamping = true;
		this.controls.dampingFactor = 0.08;
		this.controls.autoRotate = !opts.reducedMotion;
		this.controls.autoRotateSpeed = 0.35;
		this.controls.minDistance = 4;
		this.controls.maxDistance = radius * 8 + 200;
		this.controls.addEventListener('start', () => this.touch());

		const tex = glowTexture();
		this.disposables.push(tex);

		// Background starfield, far away.
		const starCount = 2500;
		const sf = new Float32Array(starCount * 3);
		for (let i = 0; i < starCount; i++) {
			const r = 600 + Math.random() * 1200;
			const t = Math.random() * Math.PI * 2;
			const u = Math.random() * 2 - 1;
			const s = Math.sqrt(1 - u * u);
			sf.set([r * s * Math.cos(t), r * u, r * s * Math.sin(t)], i * 3);
		}
		const sfGeo = new BufferGeometry();
		sfGeo.setAttribute('position', new BufferAttribute(sf, 3));
		const sfMat = new PointsMaterial({ size: 2.2, map: tex, transparent: true, depthWrite: false, color: 0x8899bb, opacity: 0.7, sizeAttenuation: true });
		this.scene.add(new Points(sfGeo, sfMat));
		this.disposables.push(sfGeo, sfMat);

		// Notes: a solid core per note plus an additive glow sprite.
		const n = opts.nodes.length;
		this.baseColors = opts.nodes.map((node) => new Color(opts.colors.get(node.group) ?? '#7dd3fc'));
		this.sizes = opts.nodes.map((node) => 0.55 + Math.min(1.4, Math.sqrt(node.degree) * 0.35));
		const sphere = new SphereGeometry(1, 20, 14);
		const coreMat = new MeshBasicMaterial({ color: 0xffffff });
		this.stars = new InstancedMesh(sphere, coreMat, Math.max(1, n));
		this.stars.count = n;
		this.disposables.push(sphere, coreMat);
		const glowPos = new Float32Array(n * 3);
		const glowCol = new Float32Array(n * 3);
		const m = new Matrix4();
		const q = new Quaternion();
		for (let i = 0; i < n; i++) {
			const p = this.at(i);
			const s = this.sizes[i];
			m.compose(p, q, new Vector3(s, s, s));
			this.stars.setMatrixAt(i, m);
			this.stars.setColorAt(i, this.baseColors[i]);
			glowPos.set([p.x, p.y, p.z], i * 3);
			glowCol.set([this.baseColors[i].r, this.baseColors[i].g, this.baseColors[i].b], i * 3);
		}
		this.scene.add(this.stars);
		const glowGeo = new BufferGeometry();
		glowGeo.setAttribute('position', new BufferAttribute(glowPos, 3));
		glowGeo.setAttribute('color', new BufferAttribute(glowCol, 3));
		const glowMat = new PointsMaterial({ size: 9, map: tex, vertexColors: true, transparent: true, opacity: 0.85, blending: AdditiveBlending, depthWrite: false, sizeAttenuation: true });
		this.glow = new Points(glowGeo, glowMat);
		this.scene.add(this.glow);
		this.disposables.push(glowGeo, glowMat);

		// Links: all of them faint, the selected note's bright.
		const linkPos = new Float32Array(opts.links.length * 6);
		opts.links.forEach((l, i) => {
			const a = this.index.get(l.source);
			const b = this.index.get(l.target);
			if (a === undefined || b === undefined) return;
			const pa = this.at(a);
			const pb = this.at(b);
			linkPos.set([pa.x, pa.y, pa.z, pb.x, pb.y, pb.z], i * 6);
		});
		const lineGeo = new BufferGeometry();
		lineGeo.setAttribute('position', new BufferAttribute(linkPos, 3));
		const lineMat = new LineBasicMaterial({ color: 0x6b8bb5, transparent: true, opacity: 0.22, depthWrite: false });
		this.lines = new LineSegments(lineGeo, lineMat);
		this.scene.add(this.lines);
		const hotGeo = new BufferGeometry();
		hotGeo.setAttribute('position', new BufferAttribute(new Float32Array(0), 3));
		const hotMat = new LineBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.85, blending: AdditiveBlending, depthWrite: false });
		this.hot = new LineSegments(hotGeo, hotMat);
		this.scene.add(this.hot);
		this.disposables.push(lineGeo, lineMat, hotGeo, hotMat);

		// One floating label for the hovered or selected note.
		this.label = document.createElement('div');
		this.label.className = 'galaxy-label';
		this.label.setAttribute('aria-hidden', 'true');
		Object.assign(this.label.style, {
			position: 'absolute',
			pointerEvents: 'none',
			transform: 'translate(-50%, -150%)',
			whiteSpace: 'nowrap',
			display: 'none'
		});
		container.appendChild(this.label);

		const el = this.renderer.domElement;
		el.addEventListener('pointermove', this.onMove);
		el.addEventListener('pointerdown', this.onDown);
		el.addEventListener('pointerup', this.onUp);
		el.addEventListener('pointerleave', this.onLeave);

		this.ro = new ResizeObserver(() => this.resize());
		this.ro.observe(container);
		this.resize();
	}

	private at(i: number): Vector3 {
		const p = this.opts.positions;
		return new Vector3(p[i * 3], p[i * 3 + 1], p[i * 3 + 2]);
	}

	private boundingRadius(ids?: Iterable<number>): number {
		const list = ids ? [...ids].map((id) => this.index.get(id)).filter((i): i is number => i !== undefined) : this.opts.nodes.map((_, i) => i);
		if (list.length === 0) return 10;
		const c = this.centroid(list);
		return Math.max(6, ...list.map((i) => this.at(i).distanceTo(c)));
	}

	private centroid(list: number[]): Vector3 {
		const c = new Vector3();
		for (const i of list) c.add(this.at(i));
		return list.length ? c.divideScalar(list.length) : c;
	}

	private touch(): void {
		this.lastInteraction = performance.now();
		this.controls.autoRotate = false;
	}

	start(): void {
		const frame = (t: number) => {
			this.raf = requestAnimationFrame(frame);
			if (this.fly) {
				const k = Math.min(1, (t - this.fly.start) / FLY_MS);
				const e = ease(k);
				this.camera.position.lerpVectors(this.fly.from, this.fly.to, e);
				this.controls.target.lerpVectors(this.fly.fromT, this.fly.toT, e);
				if (k >= 1) this.fly = null;
			} else if (!this.opts.reducedMotion && !this.controls.autoRotate && performance.now() - this.lastInteraction > IDLE_RESUME_MS && this.selected === null) {
				this.controls.autoRotate = true;
			}
			this.controls.update();
			this.renderer.render(this.scene, this.camera);
			this.placeLabel();
		};
		this.raf = requestAnimationFrame(frame);
	}

	private resize(): void {
		const w = Math.max(1, this.container.clientWidth);
		const h = Math.max(1, this.container.clientHeight);
		this.renderer.setSize(w, h, false);
		this.renderer.domElement.style.width = '100%';
		this.renderer.domElement.style.height = '100%';
		this.camera.aspect = w / h;
		// With a panel over the right side, render as if the canvas were wider, so the point the
		// camera looks at sits in the middle of the part that is still visible.
		const inset = Math.min(this.insetRight, w * 0.7);
		if (inset > 0) this.camera.setViewOffset(w + inset, h, inset, 0, w, h);
		else this.camera.clearViewOffset();
		this.camera.updateProjectionMatrix();
	}

	/** The note panel covers `px` pixels on the right (0 when closed or on a phone). */
	setInsetRight(px: number): void {
		if (px === this.insetRight) return;
		this.insetRight = Math.max(0, px);
		this.resize();
	}

	private pick(ev: PointerEvent): number | null {
		const r = this.renderer.domElement.getBoundingClientRect();
		this.pointer.set(((ev.clientX - r.left) / r.width) * 2 - 1, -((ev.clientY - r.top) / r.height) * 2 + 1);
		this.raycaster.setFromCamera(this.pointer, this.camera);
		const hit = this.raycaster.intersectObject(this.stars, false)[0];
		if (hit?.instanceId !== undefined) return this.opts.nodes[hit.instanceId]?.id ?? null;
		// Small stars are hard to hit: accept the nearest one within a few pixels on screen.
		let best: number | null = null;
		let bestD = 14;
		const v = new Vector3();
		this.opts.nodes.forEach((node, i) => {
			v.copy(this.at(i)).project(this.camera);
			if (v.z > 1) return;
			const x = ((v.x + 1) / 2) * r.width;
			const y = ((1 - v.y) / 2) * r.height;
			const d = Math.hypot(x - (ev.clientX - r.left), y - (ev.clientY - r.top));
			if (d < bestD) {
				bestD = d;
				best = node.id;
			}
		});
		return best;
	}

	private onMove = (ev: PointerEvent) => {
		const id = this.pick(ev);
		if (id !== this.hovered) {
			this.hovered = id;
			this.renderer.domElement.style.cursor = id === null ? 'grab' : 'pointer';
			this.opts.onHover?.(id);
		}
	};
	private onDown = (ev: PointerEvent) => {
		this.down = { x: ev.clientX, y: ev.clientY };
		this.touch();
	};
	private onUp = (ev: PointerEvent) => {
		const d = this.down;
		this.down = null;
		if (!d || Math.hypot(ev.clientX - d.x, ev.clientY - d.y) > 5) return; // a drag, not a click
		this.opts.onSelect(this.pick(ev));
	};
	private onLeave = () => {
		this.hovered = null;
		this.opts.onHover?.(null);
	};

	/** Light `id` and its neighbours, dim the rest, and fly to it. `null` clears. */
	select(id: number | null, neighbours: Set<number> = new Set(), fly = true): void {
		this.selected = id;
		this.paint();
		const hot: number[] = [];
		if (id !== null) {
			const a = this.index.get(id);
			for (const nb of neighbours) {
				const b = this.index.get(nb);
				if (a === undefined || b === undefined) continue;
				const pa = this.at(a);
				const pb = this.at(b);
				hot.push(pa.x, pa.y, pa.z, pb.x, pb.y, pb.z);
			}
			if (fly && a !== undefined) this.flyTo(this.at(a), 26 + this.sizes[a] * 10);
		}
		this.hot.geometry.setAttribute('position', new BufferAttribute(new Float32Array(hot), 3));
		this.hot.geometry.computeBoundingSphere();
	}

	/** Fly so the whole group of notes is in view (an answer drawn from several notes). */
	flyToCluster(ids: number[]): void {
		const list = ids.map((id) => this.index.get(id)).filter((i): i is number => i !== undefined);
		if (list.length === 0) return;
		this.highlight = new Set(ids);
		this.selected = null;
		this.paint();
		this.flyTo(this.centroid(list), this.boundingRadius(ids) * 2.6 + 8);
	}

	/** Dim every note not in `ids` (search); `null` shows all. */
	setHighlight(ids: Set<number> | null): void {
		this.highlight = ids;
		this.paint();
	}

	/** Back to the overview. */
	reset(): void {
		this.selected = null;
		this.highlight = null;
		this.paint();
		this.hot.geometry.setAttribute('position', new BufferAttribute(new Float32Array(0), 3));
		const r = this.boundingRadius();
		this.flyTo(new Vector3(), r * 2.4 + 20);
	}

	private flyTo(target: Vector3, distance: number): void {
		this.touch();
		const dir = this.camera.position.clone().sub(this.controls.target).normalize();
		if (dir.lengthSq() === 0) dir.set(0, 0, 1);
		const to = target.clone().add(dir.multiplyScalar(distance));
		if (this.opts.reducedMotion) {
			this.camera.position.copy(to);
			this.controls.target.copy(target);
			return;
		}
		this.fly = {
			from: this.camera.position.clone(),
			to,
			fromT: this.controls.target.clone(),
			toT: target.clone(),
			start: performance.now()
		};
	}

	private paint(): void {
		const dim = new Color(0x1a2233);
		const glowCol = this.glow.geometry.getAttribute('color') as BufferAttribute;
		const m = new Matrix4();
		const q = new Quaternion();
		this.opts.nodes.forEach((node, i) => {
			const lit =
				this.selected === null && this.highlight === null
					? true
					: this.selected === node.id || (this.highlight?.has(node.id) ?? false) || this.isNeighbourOfSelected(node.id);
			const c = lit ? this.baseColors[i] : this.baseColors[i].clone().lerp(dim, 0.8);
			this.stars.setColorAt(i, c);
			glowCol.setXYZ(i, c.r, c.g, c.b);
			const s = this.sizes[i] * (node.id === this.selected ? 1.4 : 1);
			m.compose(this.at(i), q, new Vector3(s, s, s));
			this.stars.setMatrixAt(i, m);
		});
		this.stars.instanceMatrix.needsUpdate = true;
		if (this.stars.instanceColor) this.stars.instanceColor.needsUpdate = true;
		glowCol.needsUpdate = true;
		(this.lines.material as LineBasicMaterial).opacity = this.selected === null && this.highlight === null ? 0.22 : 0.08;
	}

	private neighbourCache: { id: number; set: Set<number> } | null = null;
	private isNeighbourOfSelected(id: number): boolean {
		if (this.selected === null) return false;
		if (this.neighbourCache?.id !== this.selected) {
			const set = new Set<number>();
			for (const l of this.opts.links) {
				if (l.source === this.selected) set.add(l.target);
				if (l.target === this.selected) set.add(l.source);
			}
			this.neighbourCache = { id: this.selected, set };
		}
		return this.neighbourCache.set.has(id);
	}

	private placeLabel(): void {
		const id = this.hovered ?? this.selected;
		const i = id === null ? undefined : this.index.get(id);
		if (i === undefined) {
			this.label.style.display = 'none';
			return;
		}
		const v = this.at(i).project(this.camera);
		if (v.z > 1) {
			this.label.style.display = 'none';
			return;
		}
		const w = this.container.clientWidth;
		const h = this.container.clientHeight;
		this.label.textContent = this.opts.nodes[i].title;
		this.label.style.left = `${((v.x + 1) / 2) * w}px`;
		this.label.style.top = `${((1 - v.y) / 2) * h}px`;
		this.label.style.display = 'block';
	}

	dispose(): void {
		cancelAnimationFrame(this.raf);
		this.ro.disconnect();
		const el = this.renderer.domElement;
		el.removeEventListener('pointermove', this.onMove);
		el.removeEventListener('pointerdown', this.onDown);
		el.removeEventListener('pointerup', this.onUp);
		el.removeEventListener('pointerleave', this.onLeave);
		el.removeEventListener('webglcontextlost', this.onLost);
		this.controls.dispose();
		this.stars.dispose();
		for (const d of this.disposables) d.dispose();
		this.renderer.dispose();
		// Give the GPU context back now; browsers cap how many a page may hold.
		this.renderer.forceContextLoss();
		el.remove();
		this.label.remove();
	}
}
