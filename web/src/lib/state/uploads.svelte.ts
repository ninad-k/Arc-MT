import { api } from '#lib/api.ts';
import { describeError, toApiError } from '#lib/errors.ts';
import {
	fitWithin,
	MAX_CLAUDE_IMAGE_BYTES,
	MAX_FILE_BYTES,
	MAX_IMAGE_BYTES,
	uploadKind
} from '#lib/model3d/uploads.ts';
import { assistant } from './assistant.svelte';
import { chat } from './chat.svelte';

/** Shrink a picture to at most MAX_IMAGE_SIDE on its long side and `maxBytes`. PNG stays PNG
 * (plans and drawings keep sharp lines) unless that is too large; then it becomes JPEG. */
async function shrink(file: File, maxBytes: number): Promise<Blob> {
	const bitmap = await createImageBitmap(file);
	const { width, height } = fitWithin(bitmap.width, bitmap.height);
	if (width === bitmap.width && height === bitmap.height && file.size <= maxBytes) {
		bitmap.close();
		return file;
	}
	const canvas = document.createElement('canvas');
	canvas.width = width;
	canvas.height = height;
	canvas.getContext('2d')!.drawImage(bitmap, 0, 0, width, height);
	bitmap.close();
	const encode = (type: string, quality: number) =>
		new Promise<Blob>((resolve, reject) =>
			canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('the picture could not be read'))), type, quality)
		);
	let blob = await encode(file.type === 'image/png' ? 'image/png' : 'image/jpeg', 0.9);
	if (blob.size > maxBytes) blob = await encode('image/jpeg', 0.82);
	if (blob.size > maxBytes) throw new Error('the picture is too large even after shrinking');
	return blob;
}

/**
 * Uploads in Assistant mode (spec-model-uploads): a 3D file is stored and shown (free); a picture
 * asks first whether Claude should build a model of it (paid) or it should become a relief (free).
 */
class UploadStore {
	/** A picture waiting for the user's choice. */
	pendingImage = $state<{ file: File; url: string } | null>(null);
	/** Something is being uploaded or built: a line for the user. */
	busy = $state<string | null>(null);

	/** Take a file the user picked or dropped. */
	async open(file: File): Promise<void> {
		if (this.busy) {
			chat.note(`${file.name} was not opened: wait for the current upload to finish.`, 'warn');
			return;
		}
		const kind = uploadKind(file);
		if (kind === null) {
			chat.note(`${file.name} cannot be shown in 3D. Use .glb, .gltf, .obj, .stl, .ply, or a PNG, JPEG or WebP picture.`, 'warn');
			return;
		}
		if (kind === 'image') {
			if (file.size > 20 * 1024 * 1024) {
				chat.note(`${file.name} is too large (the limit for pictures is 20 MB before shrinking).`, 'warn');
				return;
			}
			this.cancelImage();
			this.pendingImage = { file, url: URL.createObjectURL(file) };
			return;
		}
		if (file.size > MAX_FILE_BYTES) {
			chat.note(`${file.name} is too large (the limit for 3D files is 25 MB).`, 'warn');
			return;
		}
		this.busy = `Uploading ${file.name}…`;
		try {
			const model = await api.uploadModel(file, file.name);
			chat.addModels(`Opened ${file.name} in 3D.`, [model]);
			assistant.show(model);
			void assistant.loadRecent();
		} catch (e) {
			chat.note(`${file.name} was not opened: ${describeError(toApiError(e))}`, 'bad');
		} finally {
			this.busy = null;
		}
	}

	cancelImage(): void {
		if (this.pendingImage) URL.revokeObjectURL(this.pendingImage.url);
		this.pendingImage = null;
	}

	/** The picture as a raised relief: free, made in the browser's sandbox. */
	async relief(): Promise<void> {
		const pending = this.pendingImage;
		if (!pending || this.busy) return;
		this.cancelImage();
		this.busy = `Making a relief of ${pending.file.name}…`;
		try {
			const image = await shrink(pending.file, MAX_IMAGE_BYTES);
			const model = await api.uploadRelief(image, pending.file.name);
			chat.addModels(`Made a 3D relief of ${pending.file.name}.`, [model]);
			assistant.show(model);
			void assistant.loadRecent();
		} catch (e) {
			chat.note(`No relief was made: ${describeError(toApiError(e))}`, 'bad');
		} finally {
			this.busy = null;
		}
	}

	/** Claude builds a 3D model of what the picture shows (paid, through the monthly cap). */
	async build(note: string): Promise<void> {
		const pending = this.pendingImage;
		if (!pending || this.busy) return;
		this.cancelImage();
		this.busy = 'Claude is building a 3D model from your picture. This can take a minute or two…';
		chat.addUser(note.trim() ? `[picture: ${pending.file.name}] ${note.trim()}` : `[picture: ${pending.file.name}] Build a 3D model of this.`);
		try {
			const image = await shrink(pending.file, MAX_CLAUDE_IMAGE_BYTES);
			const res = await api.modelFromImage(image, note);
			if (res.spend && assistant.status) assistant.status = { ...assistant.status, spend: res.spend };
			if (res.model) {
				chat.addModels(res.message, [res.model]);
				assistant.show(res.model);
				void assistant.loadRecent();
			} else {
				chat.note(res.message, 'warn');
			}
		} catch (e) {
			const err = toApiError(e);
			if (err.code === 'timeout' || err.status === 408) {
				// The build carries on on the server and may still finish (and is paid for).
				chat.note(
					'The build is taking longer than this page waits. It may still finish: check the model list at the top of the stage in a minute or two.',
					'warn'
				);
				setTimeout(() => void assistant.loadRecent(), 60_000);
			} else {
				chat.note(`No model was built: ${describeError(err)}`, 'bad');
			}
		} finally {
			this.busy = null;
		}
	}
}

export const uploads = new UploadStore();
