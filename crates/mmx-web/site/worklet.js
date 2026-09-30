// mmxrs audio ring: AudioWorkletProcessor fed interleaved stereo Float32Array
// chunks from app.js. Shrunk from z2rs z2-web/site/worklet.js: the queue IS
// the latency, so past MAX_MS we drop the oldest audio back to TARGET_MS, and
// after a full drain we re-prime to TARGET_MS before playing again. Reports
// {queued (frames), underruns} every quantum so app.js can pace on it.

const TARGET_MS = 50;
const MAX_MS = 90;

class MmxRing extends AudioWorkletProcessor {
  constructor() {
    super();
    this.target = Math.round((sampleRate * TARGET_MS) / 1000);
    this.max = Math.round((sampleRate * MAX_MS) / 1000);
    this.queue = []; // Float32Array chunks, interleaved L,R
    this.off = 0; // read offset (samples, not frames) into queue[0]
    this.pending = 0; // unplayed frames
    this.underruns = 0;
    this.priming = true;
    this.port.onmessage = (e) => {
      const c = e.data;
      if (!c || !c.length) return;
      this.queue.push(c);
      this.pending += c.length >> 1;
      if (this.pending > this.max) this.drop(this.pending - this.target);
    };
  }

  drop(frames) {
    while (frames > 0 && this.queue.length) {
      const avail = (this.queue[0].length - this.off) >> 1;
      if (avail > frames) { this.off += frames * 2; this.pending -= frames; return; }
      this.queue.shift(); this.off = 0; this.pending -= avail; frames -= avail;
    }
  }

  process(_in, outputs) {
    const [l, r] = outputs[0];
    const n = l.length;
    if (this.priming && this.pending < this.target) {
      l.fill(0); if (r) r.fill(0);
      this.port.postMessage({ queued: this.pending, underruns: this.underruns });
      return true;
    }
    this.priming = false;
    let i = 0;
    while (i < n) {
      const head = this.queue[0];
      if (!head) {
        l.fill(0, i); if (r) r.fill(0, i);
        this.underruns++; this.priming = true;
        break;
      }
      for (; i < n && this.off < head.length; i++, this.off += 2) {
        l[i] = head[this.off];
        if (r) r[i] = head[this.off + 1];
        this.pending--;
      }
      if (this.off >= head.length) { this.queue.shift(); this.off = 0; }
    }
    // ponytail: a message per 128-frame quantum (~375/s) is cheap enough and
    // saves the page from guessing the depth.
    this.port.postMessage({ queued: this.pending, underruns: this.underruns });
    return true;
  }
}

registerProcessor('mmx-ring', MmxRing);
