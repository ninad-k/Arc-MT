import { describe, expect, it } from 'vitest';
import { fitWithin, uploadKind } from '#lib/model3d/uploads.ts';
import { fileFormat } from '#lib/model3d/protocol.ts';

describe('uploadKind', () => {
	it('tells 3D files from pictures and refuses the rest', () => {
		expect(uploadKind({ name: 'car.GLB', type: '' })).toBe('model');
		expect(uploadKind({ name: 'scan.ply', type: 'application/octet-stream' })).toBe('model');
		expect(uploadKind({ name: 'plan.png', type: 'image/png' })).toBe('image');
		expect(uploadKind({ name: 'photo.JPG', type: '' })).toBe('image');
		expect(uploadKind({ name: 'anim.gif', type: 'image/gif' })).toBeNull();
		expect(uploadKind({ name: 'setup.exe', type: 'application/x-msdownload' })).toBeNull();
		expect(uploadKind({ name: 'scene.fbx', type: '' })).toBeNull();
	});
});

describe('fitWithin', () => {
	it('shrinks the long side to the limit and never enlarges', () => {
		expect(fitWithin(4000, 3000)).toEqual({ width: 1568, height: 1176 });
		expect(fitWithin(1000, 3136)).toEqual({ width: 500, height: 1568 });
		expect(fitWithin(800, 600)).toEqual({ width: 800, height: 600 });
		expect(fitWithin(0, 0)).toEqual({ width: 1, height: 1 });
	});
});

describe('fileFormat', () => {
	it('knows the formats the sandbox can load', () => {
		expect(fileFormat('a.gltf')).toBe('gltf');
		expect(fileFormat('a.stl')).toBe('stl');
		expect(fileFormat('a.glb.txt')).toBeNull();
	});
});
