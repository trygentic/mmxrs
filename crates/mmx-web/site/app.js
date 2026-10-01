// mmxrs browser shell: ROM drop, canvas blit, AudioWorklet feed, keyboard.
// Plain ES module, no bundler; loads the wasm-pack `--target web` output from
// ./pkg/. Shrunk from z2rs crates/z2-web/site/app.js. See docs/web.md.
import init, { Emu, key_to_pad } from './pkg/mmx_web.js';

const wasm = await init();
const $ = (id) => document.getElementById(id);
const canvas = $('screen');
const ctx = canvas.getContext('2d');

const FF_FRAMES = 4; // frames per rAF tick while Tab is held
const TARGET_MS = 50; // matches worklet.js

let emu = null;
let paused = false;
let coop = new URLSearchParams(location.search).get('coop') === '1';
const pads = [0, 0];
let ff = false;
let slot = null; // ponytail: one in-memory save slot, no IndexedDB
let msg = '';
let net = false; // an online session exists (connecting, live, or closed until Leave)
let netAcc = 0;
// Widescreen (docs/widescreen.md): the select, or ?widescreen=16:9.
let widescreen = new URLSearchParams(location.search).get('widescreen') || 'off';
$('widescreen').value = widescreen;
$('widescreen').addEventListener('change', (e) => {
  widescreen = e.target.value;
  emu?.set_widescreen(widescreen);
  e.target.blur(); // keep the keys for the game
});
// Wide gameplay (changes the game; netplay id): the checkbox, or ?widegame=1.
// Fixed while online: the session game was built with it.
let wideGame = new URLSearchParams(location.search).get('widegame') === '1';
$('wideGame').checked = wideGame;
$('wideGame').addEventListener('change', (e) => {
  if (net) {
    e.target.checked = wideGame;
    msg = 'wide gameplay: set it before going online';
  } else {
    wideGame = e.target.checked;
    emu?.set_wide_gameplay(wideGame);
    emu?.set_overclock(overclockNow());
  }
  e.target.blur();
});
// CPU overclock (changes the game; netplay id): the select, or ?overclock=off|1.5|2.
// auto = 2x with wide gameplay (more enemies alive, more slowdown), else off.
let overclock = new URLSearchParams(location.search).get('overclock') || 'auto';
$('overclock').value = overclock;
const overclockNow = () => (overclock === 'auto' ? (wideGame ? '2' : 'off') : overclock);
$('overclock').addEventListener('change', (e) => {
  if (net) {
    e.target.value = overclock;
    msg = 'CPU overclock: set it before going online';
  } else {
    overclock = e.target.value;
    emu?.set_overclock(overclockNow());
  }
  e.target.blur();
});

// --- ROM loading -----------------------------------------------------------
async function loadRom(file) {
  try {
    const rom = new Uint8Array(await file.arrayBuffer());
    emu?.free();
    emu = null;
    net = false;
    const e = new Emu(rom);
    const hooks = e.hook_report(); // non-empty: not the PAL dump's layout
    if (hooks) console.warn(hooks);
    let refused = '';
    try { e.set_coop(coop); } catch (err) { refused = ` (${err})`; }
    if (x3) e.set_x3_rom(x3);
    if (x3 && zero) e.set_p1_zero(true);
    e.set_widescreen(widescreen);
    e.set_wide_gameplay(wideGame);
    e.set_overclock(overclockNow());
    emu = e;
    const info = JSON.parse(emu.rom_info_json());
    msg = `${info.title.trim()} v1.${info.version} sha256 ${info.sha256.slice(0, 16)}…${refused}`;
    await startAudio();
  } catch (err) {
    msg = `load failed: ${err}`;
  }
  status();
}
$('rom').addEventListener('change', (e) => e.target.files[0] && loadRom(e.target.files[0]));
// Optional X3 ROM: P2 drawn as X3 Zero, X (T for P2) swings the saber. Kept
// for the next X ROM load too. Load it before Host/Join (both peers need it).
let x3 = null;
$('x3rom').addEventListener('change', async (e) => {
  const f = e.target.files[0];
  if (!f) return;
  try {
    const bytes = new Uint8Array(await f.arrayBuffer());
    emu?.set_x3_rom(bytes);
    x3 = bytes;
    if (zero) emu?.set_p1_zero(true);
    msg = zero ? 'X3 loaded: P1 and P2 are Zero' : 'X3 loaded: P2 is Zero';
  } catch (err) {
    msg = `X3 ROM rejected: ${err}`;
  }
  status();
});
addEventListener('dragover', (e) => { e.preventDefault(); document.body.classList.add('drag'); });
addEventListener('dragleave', () => document.body.classList.remove('drag'));
addEventListener('drop', (e) => {
  e.preventDefault();
  document.body.classList.remove('drag');
  const f = e.dataTransfer.files[0];
  if (f) loadRom(f);
});

// Play as Zero (P1): needs the X3 ROM; unchecks itself without one.
let zero = false;
$('zero').addEventListener('change', (e) => {
  zero = e.target.checked && !!x3;
  e.target.checked = zero;
  if (!zero && !x3) msg = 'Play as Zero needs the Mega Man X3 ROM: load it first';
  emu?.set_p1_zero(zero);
  status();
});

$('coop').checked = coop;
$('coop').addEventListener('change', (e) => {
  coop = e.target.checked;
  pads[0] = pads[1] = 0;
  try { emu?.set_coop(coop); } catch (err) { msg = `${err}`; status(); }
  const u = new URL(location.href);
  if (coop) u.searchParams.set('coop', '1'); else u.searchParams.delete('coop');
  history.replaceState(null, '', u);
});

// --- keyboard ----------------------------------------------------------------
addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement && e.target.type === 'file') return;
  actx?.state === 'suspended' && actx.resume();
  if (net && ['F5', 'F7', 'Tab'].includes(e.code)) { e.preventDefault(); return; } // would desync
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
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w; canvas.height = h;
    // Square pixels at 3x, capped by the window (256x224 is the old 8:7).
    canvas.style.aspectRatio = `${w} / ${h}`;
    canvas.style.width = `min(100vw - 24px, ${3 * w}px)`;
  }
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
  if (!emu) return;
  if (net) {
    netFrame(dt);
    if (emu.net_live()) return;
  }
  if (paused) return;
  let n = 0;
  if (ff) {
    for (; n < FF_FRAMES; n++) stepOne();
  } else if (audioLive()) {
    // Audio clock: keep ~TARGET_MS queued in the worklet, at most 4 frames/tick.
    const want = (actx.sampleRate * TARGET_MS) / 1000;
    while (queued + postedSince < want && n < 4) { stepOne(); n++; }
    acc = 0;
  } else {
    // Fallback: wall-clock accumulator at the game's rate (PAL ROM: 50 fps).
    acc += dt * emu.fps();
    for (; acc >= 1 && n < 4; acc--, n++) stepOne();
    if (acc > 1) acc = 0; // too slow: slow down rather than spiral
  }
  if (n) blit();
}
requestAnimationFrame(loop);

// --- online co-op (bundle built with --features netplay) ----------------------
// The game keeps running locally while connecting; the session start restarts
// both peers from power-on and from then on the audio clock drives it: one
// net_tick_audio per frame of audio the worklet needs (a tick that steps
// nothing, e.g. lockstep waiting on a late pad, queues a frame of silence).
// A fixed 60 Hz here overran the worklet on the PAL ROM (50 fps): fast music.
function netFrame(dt) {
  const now = performance.now();
  if (!emu.net_live()) {
    emu.net_tick(pads[0], now); // network only
    if (emu.net_live()) { netAcc = 0; blit(); }
    return;
  }
  let shown = 0;
  if (audioLive() && !ff) {
    const want = (actx.sampleRate * TARGET_MS) / 1000;
    for (let k = 0; queued + postedSince < want && k < 4; k++) {
      shown += emu.net_tick_audio(pads[0], now);
      pushAudio();
    }
    netAcc = 0;
  } else {
    netAcc = Math.min(netAcc + dt * emu.fps(), 4);
    for (; netAcc >= 1; netAcc--) { shown += emu.net_tick(pads[0], now); pushAudio(); }
  }
  if (shown) blit();
}

let netUsed = false; // a session was started: its log can be downloaded
const netSupported = () => !!emu && typeof emu.net_connect === 'function';
function netButtons() {
  $('netHost').disabled = $('netJoin').disabled = net || !netSupported();
  $('netLeave').disabled = !net;
  $('netLog').disabled = !netSupported() || !netUsed;
}
function netLog() {
  const text = emu?.net_record?.();
  if (!text) return;
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
  a.download = `mmxrs-${$('netRoom').value.trim() || 'session'}-${Date.now()}.log`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}
function netConnect(host) {
  if (!netSupported()) { $('netStatus').textContent = emu ? 'netplay: not in this build (--features netplay)' : 'netplay: load the ROM first'; return; }
  const signal = $('netSignal').value.trim();
  if (location.protocol === 'https:' && signal.startsWith('ws://')) {
    $('netStatus').textContent = 'netplay: this page is https, so the signal URL must be wss://';
    return;
  }
  try {
    emu.net_connect(signal, $('netRoom').value.trim(), host, $('netMode').value, +$('netDelay').value, $('netIce').value);
  } catch (err) {
    $('netStatus').textContent = `netplay: ${err}`;
    return;
  }
  net = netUsed = true;
  netButtons();
}
function netLeave() {
  if (!net) return;
  emu.net_leave();
  // keep pumping ~200 ms so the goodbye leaves, then drop the session
  setTimeout(() => { emu?.net_drop?.(); net = false; netButtons(); $('netStatus').textContent = 'netplay: left'; }, 200);
}
$('netHost').addEventListener('click', () => netConnect(true));
$('netJoin').addEventListener('click', () => netConnect(false));
$('netLeave').addEventListener('click', netLeave);
$('netLog').addEventListener('click', netLog);
{
  const q = new URLSearchParams(location.search);
  for (const k of ['signal', 'room', 'ice', 'mode', 'delay']) {
    const v = q.get(k);
    if (v !== null) $(`net${k[0].toUpperCase()}${k.slice(1)}`).value = v;
  }
}
setInterval(() => {
  netButtons();
  if (net && emu) $('netStatus').textContent = `netplay: ${emu.net_status()}`;
}, 250);

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
