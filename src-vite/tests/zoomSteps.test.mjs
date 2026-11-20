import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

// Run the real zoomIn/zoomOut step logic from Image.vue against a stub state.
const source = fs.readFileSync(new URL('../src/components/Image.vue', import.meta.url), 'utf8');
const block = source.slice(source.indexOf('const ZOOM_STEPS'), source.indexOf('const zoomActual ='));
const script = ts.transpile(block, { target: ts.ScriptTarget.ES2022 });

function run(startDisplayPercent, action, dpr = 1) {
  const ref = value => ({ value });
  const state = { scale: ref([startDisplayPercent / 100 / dpr, 1]), activeImage: ref(0), maxScale: ref(10), minScale: ref(0.1),
    containerSize: ref({ width: 1000, height: 800 }), window: { devicePixelRatio: dpr } };
  let applied = null;
  const api = new Function(...Object.keys(state), 'zoomImage', script + '; return { zoomIn, zoomOut };')(...Object.values(state), (_x, _y, s) => { applied = s; });
  api[action]();
  return Math.round(applied * dpr * 1000) / 10;
}

test('zoom in walks 10,20,30,50,70,100,200 %', () => {
  const seq = []; let z = 10;
  for (let i = 0; i < 6; i++) { z = run(z, 'zoomIn'); seq.push(z); }
  assert.deepEqual(seq, [20, 30, 50, 70, 100, 200]);
});

test('zoom out walks back down and stops at 10 %', () => {
  assert.equal(run(200, 'zoomOut'), 100);
  assert.equal(run(100, 'zoomOut'), 70);
  assert.equal(run(30, 'zoomOut'), 20);
  assert.equal(run(10, 'zoomOut'), 10);
});

test('from an arbitrary fit zoom the next step is used', () => {
  assert.equal(run(37, 'zoomIn'), 50);
  assert.equal(run(37, 'zoomOut'), 30);
});

test('steps are in real screen pixels on a scaled display', () => {
  assert.equal(run(100, 'zoomIn', 1.5), 200);
});
