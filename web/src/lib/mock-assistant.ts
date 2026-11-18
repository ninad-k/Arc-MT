/**
 * Mock-mode Assistant (spec-assistant-mode): a pretend Claude butler that "builds" a few 3D
 * models, captures notes and answers from the mock notes, so the page and the sandbox can be
 * developed and tested without a server or an API key.
 *
 * Flags (`?mock=...`): `noclaude` (no API key), `budget` (the monthly cap is used up).
 */
import type {
	AssistantStatus,
	CapturedNote,
	ChatRequest,
	ChatResponse,
	Model3d,
	ModelRef,
	ModelSummary,
	SpendStatus,
	ToolCall,
	UploadedModel,
	ImageBuild
} from './api';
import { addCapture, notesReply } from './mock-notes';
import { ApiError } from './errors';

const JET_ENGINE = `
const metal = new THREE.MeshStandardMaterial({ color: '#9aa4b2', metalness: 0.85, roughness: 0.3 });
const dark = new THREE.MeshStandardMaterial({ color: '#2b313b', metalness: 0.6, roughness: 0.5 });
const engine = new THREE.Group();
engine.name = 'engine';
const nacelle = new THREE.Mesh(new THREE.CylinderGeometry(2, 1.6, 7, 48, 1, true), metal);
nacelle.rotation.z = Math.PI / 2;
engine.add(nacelle);
const fan = new THREE.Group();
fan.position.x = 3.2;
for (let i = 0; i < 18; i++) {
  const blade = new THREE.Mesh(new THREE.BoxGeometry(0.08, 1.7, 0.35), dark);
  blade.position.y = 0.95;
  const holder = new THREE.Group();
  holder.rotation.x = (i / 18) * Math.PI * 2;
  holder.add(blade);
  fan.add(holder);
}
const hub = new THREE.Mesh(new THREE.ConeGeometry(0.6, 1.2, 32), metal);
hub.rotation.z = -Math.PI / 2;
hub.position.x = 0.4;
fan.add(hub);
engine.add(fan);
const exhaust = new THREE.Mesh(new THREE.CylinderGeometry(1.1, 0.8, 1.6, 32), dark);
exhaust.rotation.z = Math.PI / 2;
exhaust.position.x = -4.2;
engine.add(exhaust);
scene.add(engine);
const tag = helpers.label('Fan', { size: 0.6 });
tag.position.set(3.2, 2.8, 0);
scene.add(tag);
return (t, dt) => { fan.rotation.x += dt * 6; };
`;

const SOLAR = `
const sun = new THREE.Mesh(new THREE.SphereGeometry(1.6, 48, 24), new THREE.MeshStandardMaterial({ color: '#fbbf24', emissive: '#f59e0b', emissiveIntensity: 1.2 }));
scene.add(sun);
const planets = [];
[['#94a3b8', 0.25, 3], ['#fcd34d', 0.4, 4.4], ['#38bdf8', 0.45, 6], ['#f87171', 0.35, 7.6], ['#fdba74', 0.9, 10]].forEach(([c, r, d], i) => {
  const p = new THREE.Mesh(new THREE.SphereGeometry(r, 32, 16), new THREE.MeshStandardMaterial({ color: c }));
  const orbit = new THREE.Group();
  p.position.x = d;
  orbit.add(p);
  orbit.rotation.y = i;
  scene.add(orbit);
  const ring = new THREE.Mesh(new THREE.TorusGeometry(d, 0.01, 6, 128), new THREE.MeshBasicMaterial({ color: '#334155' }));
  ring.rotation.x = Math.PI / 2;
  scene.add(ring);
  planets.push([orbit, 0.6 / (i + 1)]);
});
return (t, dt) => planets.forEach(([o, s]) => (o.rotation.y += dt * s));
`;

const CUBE = `
const mat = new THREE.MeshStandardMaterial({ color: '#ef4444', metalness: 0.2, roughness: 0.4 });
scene.add(new THREE.Mesh(new THREE.BoxGeometry(4, 4, 4), mat));
`;

/** Code that tries to escape the sandbox; every line must fail inside it (end-to-end test). */
const ESCAPE = `
const results = [];
try { results.push('parent:' + window.parent.document.title); } catch (e) { results.push('parent blocked'); }
try { localStorage.setItem('x', '1'); results.push('storage open'); } catch (e) { results.push('storage blocked'); }
fetch('/api/killswitch', { method: 'POST', body: '{"engaged":true}' }).catch(() => {});
throw new Error(results.join(', '));
`;

const models = new Map<string, Model3d>();
let spent = 0;
let counter = 0;

/** The mock flags, read the way mock.ts reads them (`?mock=a,b`, remembered for the tab). */
function flag(name: string): boolean {
	let raw: string | null = null;
	try {
		const q = new URLSearchParams(location.search).get('mock');
		raw = q === 'reset' ? null : (q ?? sessionStorage.getItem('jarvis.mock.flags'));
	} catch {
		raw = null;
	}
	return (raw ?? '').split(',').includes(name);
}

function spend(): SpendStatus {
	const blocked = flag('budget');
	return { month: '2026-11', spent_usd: blocked ? 20.01 : spent, budget_usd: 20, warn: blocked, blocked };
}

export function assistantStatus(): AssistantStatus {
	const available = !flag('noclaude');
	return {
		enabled: true,
		available,
		reason: available ? null : 'missing API key: ANTHROPIC_API_KEY is not set',
		brain: { provider: 'claude', model: 'claude-opus-5-5' },
		spend: spend(),
		notes: true
	};
}

function build(description: string, previous?: Model3d): ModelRef {
	const d = description.toLowerCase();
	let title = 'Model';
	let code = CUBE;
	if (/escape|evil/.test(d)) [title, code] = ['Escape test', ESCAPE];
	else if (/forever|endless/.test(d)) [title, code] = ['Endless loop', 'while (true) {}'];
	else if (/wander|navigate/.test(d)) [title, code] = ['Wanderer', "location.href = 'about:blank';"];
	else if (/jet|engine/.test(d)) [title, code] = ['Jet engine', JET_ENGINE];
	else if (/solar|planet/.test(d)) [title, code] = ['Solar system', SOLAR];
	else if (/cube|box/.test(d)) [title, code] = ['Red cube', CUBE];
	if (previous && /blue/.test(d)) [title, code] = [`${previous.title} (blue)`, previous.code.replace(/#ef4444|#9aa4b2/g, '#3b82f6')];
	counter++;
	const id = counter.toString(16).padStart(12, '0');
	models.set(id, { id, title, description, code, created_at: new Date().toISOString(), previous_id: previous?.id ?? null, kind: 'code' });
	return { id, title };
}

export function listModels(): { models: ModelSummary[] } {
	const list = [...models.values()]
		.reverse()
		.map(({ id, title, description, created_at, kind }) => ({ id, title, description, created_at, kind }));
	return { models: list };
}

// ---- Uploads (spec-model-uploads) ----------------------------------------------------------

const files = new Map<string, ArrayBuffer>();

const FLOOR_PLAN = `
const floor = new THREE.Mesh(new THREE.BoxGeometry(8, 0.1, 6), new THREE.MeshStandardMaterial({ color: '#d6c7a1' }));
scene.add(floor);
const wall = new THREE.MeshStandardMaterial({ color: '#e5e7eb' });
[[0, -3, 8, 0.15], [0, 3, 8, 0.15], [-4, 0, 0.15, 6], [4, 0, 0.15, 6], [1, 0, 0.15, 6]].forEach(([x, z, w, d]) => {
  const m = new THREE.Mesh(new THREE.BoxGeometry(w, 3, d), wall);
  m.position.set(x, 1.5, z);
  scene.add(m);
});
const living = helpers.label('Living room', { size: 0.5 });
living.position.set(-1.5, 3.4, 0);
scene.add(living);
const bed = helpers.label('Bedroom', { size: 0.5 });
bed.position.set(2.5, 3.4, 0);
scene.add(bed);
`;

function nextId(): string {
	counter++;
	return counter.toString(16).padStart(12, '0');
}

const FORMATS = ['glb', 'gltf', 'obj', 'stl', 'ply'];

function titleFrom(name: string): string {
	const stem = (name.split(/[\\/]/).pop() ?? name).replace(/\.[^.]*$/, '');
	return stem.split(/[^\p{L}\p{N}]+/u).filter(Boolean).join(' ').slice(0, 60) || 'Uploaded model';
}

/** `POST /api/models/upload` and `/relief` in mock mode. */
export async function upload(kind: 'file' | 'relief', name: string, blob: Blob | undefined): Promise<UploadedModel> {
	const ext = name.toLowerCase().split('.').pop() ?? '';
	const format = kind === 'file' ? ext : ext === 'jpg' ? 'jpeg' : ext;
	if (!blob || blob.size === 0) throw new ApiError('bad_request', 'the file is empty', 400);
	if (kind === 'file' && !FORMATS.includes(ext)) throw new ApiError('bad_request', `.${ext} files are not supported (use .glb, .gltf, .obj, .stl or .ply)`, 400);
	const id = nextId();
	const title = titleFrom(name);
	files.set(id, await blob.arrayBuffer());
	models.set(id, {
		id,
		title,
		description: kind === 'file' ? `Uploaded file ${title}` : `Relief of ${title}`,
		code: '',
		created_at: new Date().toISOString(),
		previous_id: null,
		kind,
		format,
		info: null,
		overlay: null
	});
	return { id, title, kind, format };
}

/** `POST /api/models/from-image` in mock mode: a pretend Claude build of a floor plan. */
export function fromImage(note: string): ImageBuild {
	if (flag('noclaude')) return { model: null, message: 'Claude is not set up (missing API key). Set ANTHROPIC_API_KEY and restart Arc MT.', spend: spend() };
	if (flag('budget')) return { model: null, message: 'The monthly Claude budget is used up, sir.', spend: spend() };
	spent += 0.15;
	const id = nextId();
	const title = '1 BHK flat';
	models.set(id, { id, title, description: `Built from an uploaded image: ${note}`, code: FLOOR_PLAN, created_at: new Date().toISOString(), previous_id: null, kind: 'code', format: null, info: null, overlay: null });
	return { model: { id, title }, message: `Built "${title}" from your image.`, spend: spend() };
}

export function modelFile(id: string): Blob | null {
	const bytes = files.get(id);
	return bytes ? new Blob([bytes]) : null;
}

export function putInfo(id: string, info: Model3d['info']): boolean {
	const m = models.get(id);
	if (!m) return false;
	m.info = info;
	return true;
}

export function getModel(id: string): Model3d | null {
	return models.get(id) ?? null;
}

const base = (req: ChatRequest, reply: string): ChatResponse => ({
	session_id: req.session_id ?? '00000000-0000-4000-8000-00000000a551',
	reply,
	provider: 'claude',
	persona: 'jarvis',
	tool_calls: [],
	notes: [],
	mode: 'assistant',
	brain: { provider: 'claude', model: 'claude-opus-5-5' },
	models: [],
	captured: [],
	spend: spend()
});

/** One assistant turn in mock mode. */
export function assistantChat(req: ChatRequest): ChatResponse {
	if (flag('noclaude'))
		return base(req, 'Assistant mode needs Claude, and it is not set up yet (missing API key). Set ANTHROPIC_API_KEY in the server\'s environment or .env file, then restart Arc MT.');
	if (flag('budget')) return base(req, 'The monthly Claude budget is used up, sir: $20.01 of $20.00 spent in 2026-11.');
	spent += 0.02;
	const m = req.message.trim();
	const t = m.toLowerCase();
	const call = (name: string, args: Record<string, unknown>, ok = true): ToolCall => ({ name, arguments: args, ok, duration_ms: 5 });

	const remember = m.match(/^(?:remember that|note to self[:,]?|don't let me forget(?: that)?)\s+(.+)$/i);
	if (remember) {
		const note: CapturedNote = addCapture(remember[1]);
		return { ...base(req, `Noted, sir: ${note.title}.`), tool_calls: [call('remember_note', { text: remember[1] })], captured: [note], notes: [{ id: note.id, title: note.title }] };
	}
	const last = [...models.values()].at(-1);
	const edit = last && /\b(make it|change it)\b/.test(t) ? last : undefined;
	if (edit || /\b(3d|model|show me)\b/.test(t)) {
		spent += 0.15;
		const built = build(m, edit);
		return {
			...base(req, edit ? `As you wish, sir: ${built.title}.` : `Here is your ${built.title.toLowerCase()}, sir.`),
			tool_calls: [call('build_3d_model', edit ? { description: m, previous_id: edit.id } : { description: m })],
			models: [built],
			spend: spend()
		};
	}
	if (/\b(buy|sell|trade|position|price)\b/.test(t))
		return base(req, 'I have no access to trading in this mode, sir. The Trading switch at the top will take you there.');
	const fromNotes = notesReply(m);
	if (fromNotes) return { ...base(req, fromNotes.reply), tool_calls: fromNotes.tool_calls, notes: fromNotes.notes };
	return base(req, 'Very good, sir.');
}
