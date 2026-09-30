// mmxrs browser shell: ROM drop, canvas blit, AudioWorklet feed, keyboard.
// Plain ES module, no bundler; loads the wasm-pack `--target web` output from
// ./pkg/. Shrunk from z2rs crates/z2-web/site/app.js. See docs/web.md.
import init, { Emu, key_to_pad } from './pkg/mmx_web.js';

const wasm = await init();
const $ = (id) => document.getElementById(id);
const canvas = $('screen');
const ctx = canvas.getContext('2d');

const NTSC_HZ = 60.0988;
const FF_FRAMES = 4; // frames per rAF tick while Tab is held
const TARGET_MS = 50; // matches worklet.js

let emu = null;
let paused = false;
let coop = new URLSearchParams(location.search).get('coop') === '1';
const pads = [0, 0];
let ff = false;
let slot = null; // ponytail: one in-memory save slot, no IndexedDB
let msg = '';

// --- ROM loading -----------------------------------------------------------
async function loadRom(file) {
  try {
    const rom = new Uint8Array(await file.arrayBuffer());
    emu?.free();
    emu = null;
    const e = new Emu(rom);
    e.set_coop(coop);
    emu = e;
    const info = JSON.parse(emu.rom_info_json());
    msg = `${info.title.trim()} v1.${info.version} sha256 ${info.sha256.slice(0, 16)}…`;
    await startAudio();
  } catch (err) {
    msg = `load failed: ${err}`;
  }
  status();
}
$('rom').addEventListener('change', (e) => e.target.files[0] && loadRom(e.target.files[0]));
addEventListener('dragover', (e) => { e.preventDefault(); document.body.classList.add('drag'); });
addEventListener('dragleave', () => document.body.classList.remove('drag'));
addEventListener('drop', (e) => {
  e.preventDefault();
  document.body.classList.remove('drag');
  const f = e.dataTransfer.files[0];
  if (f) loadRom(f);
});

$('coop').checked = coop;
$('coop').addEventListener('change', (e) => {
  coop = e.target.checked;
  pads[0] = pads[1] = 0;
  emu?.set_coop(coop);
  const u = new URL(location.href);
  if (coop) u.searchParams.set('coop', '1'); else u.searchParams.delete('coop');
  history.replaceState(null, '', u);
});

// --- keyboard ----------------------------------------------------------------
addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement && e.target.type === 'file') return;
  actx?.state === 'suspended' && actx.resume();
  if (e.code === 'F5') { e.preventDefault(); if (emu) { slot = emu.save_state(); msg = 'saved'; status(); } return; }
  if (e.code === 'F7') { e.preventDefault(); if (emu && slot) { emu.load_state(slot); msg = 'loaded'; status(); } return; }
  if (e.code === 'Tab') { e.preventDefault(); ff = true; return; }
  const v = key_to_pad(e.code, coop);
  if (!v) return;
  e.preventDefault();
  pads[(v >>> 16) - 1] |= v & 0xffff;
});
addEventListener('keyup', (e) => {
  if (e.code === 'Tab') { ff = false; return; }
  const v = key_to_pad(e.code, coop);
  if (v) pads[(v >>> 16) - 1] &= ~(v & 0xffff);
});
addEventListener('blur', () => { pads[0] = pads[1] = 0; ff = false; });

// --- audio -------------------------------------------------------------------
let actx = null;
let node = null;
let queued = 0; // worklet's last report, in stereo frames
let postedSince = 0; // frames posted since that report
let underruns = 0;

async function startAudio() {
  try {
    if (!actx) {
      actx = new AudioContext({ latencyHint: 'interactive' });
      await actx.audioWorklet.addModule(new URL('./worklet.js', import.meta.url));
      node = new AudioWorkletNode(actx, 'mmx-ring', { outputChannelCount: [2] });
      node.port.onmessage = (e) => { queued = e.data.queued; underruns = e.data.underruns; postedSince = 0; };
      node.connect(actx.destination);
    }
    emu.set_audio_rate(actx.sampleRate);
    await actx.resume();
  } catch (err) {
    msg += `\naudio off: ${err}`;
  }
}
const audioLive = () => node && actx.state === 'running';

function pushAudio() {
  const pcm = emu.audio_take(); // always drain, even when muted
  if (!audioLive() || ff || !pcm.length) return;
  node.port.postMessage(pcm, [pcm.buffer]);
  postedSince += pcm.length >> 1;
}

// --- frame loop --------------------------------------------------------------
function stepOne() {
  emu.step(pads[0], coop ? pads[1] : 0);
  pushAudio();
}

function blit() {
  const w = emu.width(), h = emu.height();
  if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
  // Zero-copy view into wasm memory; rebuilt each time (memory can grow).
  const px = new Uint8ClampedArray(wasm.memory.buffer, emu.frame_ptr(), emu.frame_len());
  ctx.putImageData(new ImageData(px, w, h), 0, 0);
}

let lastT = 0;
let acc = 0;
function loop(t) {
  requestAnimationFrame(loop);
  const dt = Math.min((t - (lastT || t)) / 1000, 0.25);
  lastT = t;
  if (!emu || paused) return;
  let n = 0;
  if (ff) {
    for (; n < FF_FRAMES; n++) stepOne();
  } else if (audioLive()) {
    // Audio clock: keep ~TARGET_MS queued in the worklet, at most 4 frames/tick.
    const want = (actx.sampleRate * TARGET_MS) / 1000;
    while (queued + postedSince < want && n < 4) { stepOne(); n++; }
    acc = 0;
  } else {
    // Fallback: wall-clock accumulator at the SNES NTSC rate.
    acc += dt * NTSC_HZ;
    for (; acc >= 1 && n < 4; acc--, n++) stepOne();
    if (acc > 1) acc = 0; // too slow: slow down rather than spiral
  }
  if (n) blit();
}
requestAnimationFrame(loop);

function status() {
  const a = audioLive() ? `audio ${underruns} underruns` : 'audio off (press a key)';
  $('status').textContent = `${msg}\n${a}${coop ? ' · co-op' : ''}`;
}
setInterval(() => emu && status(), 1000);

// --- automation --------------------------------------------------------------
// window.mmx: drive the page from devtools / Playwright. `pause(true)` stops
// the rAF loop so `step` is frame-exact.
window.mmx = {
  wasm,
  get emu() { return emu; },
  pads,
  key_to_pad,
  pause(on = true) { paused = on; },
  step(n = 1, p1 = 0, p2 = 0) {
    for (let i = 0; i < n; i++) { emu.step(p1, p2); pushAudio(); }
    blit();
    return emu.frame_count();
  },
  save() { return (slot = emu.save_state()); },
  load(s = slot) { emu.load_state(s); blit(); },
  frame() { return { width: emu.width(), height: emu.height(), rgba: emu.frame_rgba() }; },
  wram() { return emu.wram(); },
  info() { return JSON.parse(emu.rom_info_json()); },
  async loadRomBytes(bytes) { await loadRom(new Blob([bytes])); },
};
