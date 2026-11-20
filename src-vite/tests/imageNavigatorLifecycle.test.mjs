import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import ts from 'typescript';

// Exercise the component's real buffer-switch and viewport calculations,
// without requiring a Tauri window or asynchronous image decoding.
const source = fs.readFileSync(new URL('../src/components/Image.vue', import.meta.url), 'utf8');
const ready = source.slice(source.indexOf('const onImageReady ='), source.indexOf('const rotateView ='));
const savedViewport = source.slice(source.indexOf('// ---- Saved viewport'), source.indexOf('function getViewportState()'));
const viewport = source.slice(source.indexOf('function getViewportState()'), source.indexOf('// Expose methods'));
const script = ts.transpile(ready + savedViewport + viewport, { target: ts.ScriptTarget.ES2022 });

function setup(deferred = false, saved = null) {
  const ref = value => ({ value });
  const state = {
    activeImage: ref(0),
    imageSize: ref([{ width: 4000, height: 2000 }, { width: 2000, height: 1000 }]),
    imageSizeRotated: ref([{ width: 4000, height: 2000 }, { width: 2000, height: 1000 }]),
    imageRotate: ref([0, 0]),
    scale: ref([1, 1]),
    position: ref([{ x: -1100, y: -800 }, { x: 0, y: 0 }]),
    containerSize: ref(deferred ? { width: 0, height: 0 } : { width: 1000, height: 800 }),
    isZoomFit: ref(false), isGrabbing: ref(true), isLoading: ref(true),
    noTransition: ref(false), suppressViewportEmit: ref(false), minScale: ref(0.1), maxScale: ref(10),
    gestureType: ref('none'), props: { fileId: 42, filePath: '/photos/a.jpg', isZoomFit: false }, loadingTimeout: null,
    imageFilePath: ref(['/photos/old.jpg', '/photos/a.jpg']), hasSavedViewport: ref(false),
    skipNextZoomFitWatch: false, zoomFitBeforeRestore: null, zoomLocked: ref(false),
  };
  const events = [];
  const toasts = [];
  let resize;
  const dependencies = {
    ...state, emit: (event, value) => { if (event !== 'saved-viewport-change') events.push({ event, value }); }, triggerRef: () => {},
    requestAnimationFrame: () => {}, setTimeout: () => {}, clearTimeout: () => {},
    watch: (_, callback) => { resize = callback; return () => {}; },
    window: { devicePixelRatio: 1 },
    getSavedViewport: () => saved, setSavedViewport: (_path, value) => { saved = value; }, removeSavedViewport: () => true,
    toast: { info: m => toasts.push(m), success: m => toasts.push(m) }, t: key => key,
    updateZoomFit: () => events.push({ event: 'zoom-fit-recalculated' }),
  };
  const api = new Function(...Object.keys(dependencies), script + '; return { onImageReady, saveViewport, restoreSavedViewport };')(...Object.values(dependencies));
  const readyImage = api.onImageReady;
  return { state, events, toasts, api, readyImage, getSaved: () => saved, resize() {
    state.containerSize.value = { width: 1000, height: 800 };
    resize(state.containerSize.value);
  } };
}

test('non-fit buffer switch publishes the new file viewport after silent restore', () => {
  const t = setup();
  t.readyImage(1);
  assert.equal(t.state.activeImage.value, 1);
  assert.equal(t.events.length, 1);
  assert.equal(t.events[0].event, 'viewport-change');
  assert.deepEqual(t.events[0].value, {
    fileId: 42, scale: 1, normX: 0.4, normY: 0.6, sourceWidth: 2000, sourceHeight: 1000,
    viewportWidth: 1000, viewportHeight: 800, rotate: 0, pannable: true,
  });
});

test('deferred layout publishes only after the new image has a viewport', () => {
  const t = setup(true);
  t.readyImage(1, true);
  assert.equal(t.state.isLoading.value, true, 'placeholder retains the loading indicator');
  assert.equal(t.events.length, 0);
  t.resize();
  assert.equal(t.events.length, 1);
  assert.equal(t.events[0].value.fileId, 42);
  assert.equal(t.events[0].value.sourceWidth, 2000);
  assert.equal(t.events[0].value.viewportWidth, 1000);
});

test('saving a viewport and restoring it on another buffer reproduces zoom and position', () => {
  const t = setup();
  // Currently shown image is slot 0 with a zoomed/panned viewport; slot 1 is the same file.
  t.state.imageFilePath.value = ['/photos/a.jpg', '/photos/a.jpg'];
  t.state.props.filePath = '/photos/a.jpg';
  t.state.scale.value[0] = 1.5;
  t.state.position.value[0] = { x: -1100, y: -800 };
  const before = t.api.saveViewport();
  assert.equal(before, true);
  const saved = t.getSaved();
  assert.equal(saved.displayScale, 1.5);
  assert.equal(saved.sourceWidth, 4000);
  const original = JSON.parse(JSON.stringify({ x: t.state.position.value[0].x, y: t.state.position.value[0].y }));

  // Reset the view, then restore from the saved entry.
  t.state.scale.value[0] = 0.2;
  t.state.position.value[0] = { x: 0, y: 0 };
  assert.equal(t.api.restoreSavedViewport(saved), true);
  assert.equal(t.state.isZoomFit.value, false);
  assert.ok(Math.abs(t.state.scale.value[0] - 1.5) < 1e-9);
  assert.ok(Math.abs(t.state.position.value[0].x - original.x) < 1e-6);
  assert.ok(Math.abs(t.state.position.value[0].y - original.y) < 1e-6);
});

test('saved viewport keeps real zoom when layout width differs from save time', () => {
  const t = setup();
  // Saved on a 8000 px wide layout at 100 %; current layout is 4000 px wide -> scale doubles.
  t.api.restoreSavedViewport({ displayScale: 1, normX: 0.5, normY: 0.5, sourceWidth: 8000, savedAt: 0 });
  assert.equal(t.state.scale.value[0], 2);
});

test('saving while the image is shown at fit scale does not store anything', () => {
  const t = setup();
  t.state.isZoomFit.value = true;
  t.state.scale.value = [0.25, 1];
  t.state.imageFilePath.value = ['/photos/a.jpg', '/photos/a.jpg'];
  assert.equal(t.api.saveViewport(), false);
  assert.equal(t.getSaved(), null);
});

test('opening a file with a saved viewport restores it even in zoom-fit mode', () => {
  const t = setup(false, { displayScale: 2, normX: 0.25, normY: 0.75, sourceWidth: 2000, savedAt: 1 });
  t.state.isZoomFit.value = true;
  t.state.props.isZoomFit = true;
  t.readyImage(1);
  assert.equal(t.state.isZoomFit.value, false);
  assert.equal(t.state.scale.value[1], 2);
  assert.ok(t.events.some(e => e.event === 'update:isZoomFit' && e.value === false));
  const vp = t.events.find(e => e.event === 'viewport-change').value;
  assert.ok(Math.abs(vp.normX - 0.25) < 1e-9 && Math.abs(vp.normY - 0.75) < 1e-9);
});

test('save viewport works after zoom-by-wheel even though the fit mode flag is still on', () => {
  const t = setup();
  t.state.isZoomFit.value = true;          // mode flag untouched by wheel zoom
  t.state.props.isZoomFit = true;
  t.state.imageFilePath.value = ['/photos/a.jpg', '/photos/a.jpg'];
  t.state.scale.value = [2, 1];            // fit scale is 0.25 for 4000x2000 in 1000x800
  assert.equal(t.api.saveViewport(), true);
  assert.ok(t.getSaved());
});

test('save viewport refuses when the image is displayed at fit scale', () => {
  const t = setup();
  t.state.imageFilePath.value = ['/photos/a.jpg', '/photos/a.jpg'];
  t.state.scale.value = [0.25, 1];
  assert.equal(t.api.saveViewport(), false);
  assert.deepEqual(t.toasts, ['image_viewer.viewport_fit_hint']);
});

test('zoom lock carries the previous zoom to the next image even in fit mode', () => {
  const t = setup();
  t.state.isZoomFit.value = true;
  t.state.props.isZoomFit = true;
  t.state.zoomLocked.value = true;
  t.state.scale.value = [1, 1];
  t.readyImage(1);
  assert.equal(t.state.scale.value[1], 1);
});
