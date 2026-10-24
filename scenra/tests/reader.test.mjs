import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

// Exercise the actual hook with deterministic timers and IPC, without recording on the test host.
function harness({ actions = [], mode = 'record', freeMemoryMb = 2048, failPress = false, setupFails = false } = {}) {
  let slots = [], cursor = 0, effects = [], dirty = true, reader, now = 0, nextId = 0;
  let paused = false;
  const timers = new Map();
  const calls = [];
  const script = { title: 'Tour', warnings: [], scenes: [{ title: 'Scene', lines: [{ text: 'Hello demo', actions }] }] };
  const session = { id: 'test-session', mode };
  const ipc = {
    words: line => line.text.split(/\s+/).filter(Boolean),
    checkSceneStart: async () => calls.push('check'),
    resetScenePage: async () => {}, approveSceneStart: async () => {},
    prepareScene: async () => { calls.push("prepare"); if (setupFails) throw Error("Covered target"); return { actionsRun: 2 }; },
    playbackGuard: async active => { calls.push(active ? 'arm' : 'disarm'); },
    playbackStatus: async () => ({ interference: false, stopped: false }),
    approachAction: async () => calls.push('approach'),
    pressAction: async () => { if (failPress) throw Error('Simulated failure'); calls.push('press'); },
  };
  const engine = {
    takePreflight: async () => ({ freeMemoryMb, warning: freeMemoryMb < 1024 ? "Low RAM: record anyway or check again" : null }),
    startTake: async name => calls.push(name),
    stopTake: async () => ({ microphoneSeconds: 5, droppedFramesPercent: 12, freeMemoryMb: 900 }),
  };
  const react = {
    useState(initial) {
      const index = cursor++;
      slots[index] ??= { value: initial };
      return [slots[index].value, value => {
        const next = typeof value === 'function' ? value(slots[index].value) : value;
        if (!Object.is(next, slots[index].value)) { slots[index].value = next; dirty = true; }
      }];
    },
    useRef(initial) { const index = cursor++; slots[index] ??= { current: initial }; return slots[index]; },
    useEffect(fn, deps) {
      const index = cursor++;
      const old = slots[index];
      if (!old || deps.some((value, i) => !Object.is(value, old.deps[i]))) {
        effects.push(() => { old?.cleanup?.(); slots[index] = { deps, cleanup: fn() }; });
      }
    },
  };
  const compiled = ts.transpileModule(fs.readFileSync(new URL('../src/reader.ts', import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const module = { exports: {} };
  vm.runInNewContext(compiled, {
    exports: module.exports,
    require: name => name === 'react' ? react : name === './engine' ? engine : ipc,
    setTimeout: (fn, delay) => { const id = ++nextId; timers.set(id, { fn, due: now + delay }); return id; },
    clearTimeout: id => timers.delete(id),
    setInterval: () => ++nextId, clearInterval: () => {},
    performance: { now: () => now }, console,
  });
  async function flush() {
    for (let i = 0; i < 30; i++) {
      if (dirty) {
        dirty = false; cursor = 0;
        reader = module.exports.useReader(script, session, paused, 'Natural');
        const pending = effects; effects = []; pending.forEach(fn => fn());
      }
      await Promise.resolve();
    }
  }
  async function advance(ms, stalled = false) {
    const end = now + ms;
    while (true) {
      const entry = [...timers].sort((a,b) => a[1].due - b[1].due)[0];
      if (!entry || entry[1].due > end) break;
      now = stalled ? end : entry[1].due;
      timers.delete(entry[0]); entry[1].fn(); await flush();
    }
    now = end; await flush();
  }
  return { flush, advance, calls, get reader() { return reader; },
    pause: async () => { paused = true; dirty = true; await flush(); } };
}

test('final scene Retry starts a new take in the same session and preserves warning data', async () => {
  const h = harness(); await h.flush();
  await h.reader.startScene(); await h.flush();
  await h.advance(8000);
  assert.equal(h.reader.finished, true);
  assert.equal(h.reader.takes[0].take.droppedFramesPercent, 12);
  await h.reader.restartScene(); await h.flush();
  assert.equal(h.reader.finished, false);
  assert.equal(h.reader.phase, 'reading');
  assert.ok(h.calls.includes('test-session/scene-01-take-2'));
  assert.equal(h.reader.takes.length, 1);
});

test('a stalled narration clock halts playback rather than emitting a late click', async () => {
  const h = harness({ actions: [{ word: 1, target: 'Demo', label: 'Click Demo' }] });
  await h.flush(); await h.reader.startScene(); await h.flush();
  await h.advance(3000);
  await h.advance(5000, true);
  assert.match(h.reader.lastError, /stalled/);
  assert.equal(h.calls.includes('press'), false);
});

test('pause prevents narration and action dispatch', async () => {
  const h = harness({ actions: [{ word: 1, target: 'Demo', label: 'Click Demo' }] });
  await h.flush(); await h.reader.startScene(); await h.flush(); await h.pause();
  await h.advance(10000);
  assert.equal(h.calls.includes('press'), false);
  assert.equal(h.reader.pos.word, 0);
});

test('low memory waits for presenter override and does not consume an attempt', async () => {
  const h = harness({ freeMemoryMb: 500 }); await h.flush();
  await h.reader.startScene(); await h.flush();
  assert.equal(h.reader.phase, 'ready');
  assert.match(h.reader.memoryWarning, /Low RAM/);
  assert.equal(h.calls.some(c => c.includes('scene-01-take')), false);
  await h.reader.startScene(undefined, true); await h.flush();
  assert.equal(h.reader.phase, 'reading');
  assert.ok(h.calls.includes('test-session/scene-01-take-1'));
});

test('action failure halts narration instead of saving a falsely successful scene', async () => {
  const h = harness({ actions: [{ word: 1, target: 'Demo', label: 'Click Demo' }], failPress: true });
  await h.flush(); await h.reader.startScene(); await h.flush(); await h.advance(8000);
  assert.equal(h.reader.halted, true);
  assert.equal(h.reader.finished, false);
  assert.match(h.reader.lastError, /Simulated failure/);
});

test('replay setup checks the result and returns ready without starting OBS', async () => {
  const h = harness(); await h.flush(); await h.reader.setupPage('replay'); await h.flush();
  assert.equal(h.reader.phase, 'ready');
  assert.match(h.reader.setupNote, /Replayed 2/);
  assert.ok(h.calls.includes('prepare'));
  assert.equal(h.calls.some(c => c.includes('scene-01-take')), false);
});

test('failed setup keeps manual recovery available without starting a take', async () => {
  const h = harness({ setupFails: true }); await h.flush(); await h.reader.setupPage('replay'); await h.flush();
  assert.equal(h.reader.phase, 'ready');
  assert.equal(h.reader.needsSetup, true);
  assert.match(h.reader.lastError, /Covered target/);
  assert.equal(h.reader.setupNote, null);
  assert.equal(h.calls.some(c => c.includes('scene-01-take')), false);
});
