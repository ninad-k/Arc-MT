/**
 * Upload rules shared by the upload flow and its tests (spec-model-uploads). Same caps as the
 * server (crates/api/src/models3d.rs).
 */
import { fileFormat } from './protocol';

export const MAX_FILE_BYTES = 25 * 1024 * 1024;
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
/** Pictures for Claude: their base64 form must stay under Anthropic's 5 MB. */
export const MAX_CLAUDE_IMAGE_BYTES = 3_750_000;
/** Claude scales larger images down anyway; smaller ones cost fewer tokens. */
export const MAX_IMAGE_SIDE = 1568;

const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp'];

/** What kind of upload a file is, by its name and type. */
export function uploadKind(file: { name: string; type: string }): 'model' | 'image' | null {
	if (fileFormat(file.name)) return 'model';
	if (IMAGE_TYPES.includes(file.type) || /\.(png|jpe?g|webp)$/i.test(file.name)) return 'image';
	return null;
}

/** The size of a picture once its long side is at most `max` (never enlarged). */
export function fitWithin(width: number, height: number, max = MAX_IMAGE_SIDE): { width: number; height: number } {
	const scale = Math.min(1, max / Math.max(width, height, 1));
	return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}
