// Runs one game module in a Worker: the module can compute and nothing else,
// because it has no imports and a Worker has no page. module-host.js is the
// other side, and its header has the protocol.

const DATA = 64;

self.onmessage = ({ data: { module, sab } }) => {
  let x;
  try {
    x = new WebAssembly.Instance(module, {}).exports;
  } catch (e) {
    self.postMessage({ error: String(e?.message ?? e) });
    return;
  }
  self.postMessage({ names: Object.keys(x) });
  const ctl = new Int32Array(sab, 0, 8);
  const num = new Float64Array(sab, 32, 2);
  const view = new DataView(sab);
  const bytes = new Uint8Array(sab);
  const capacity = sab.byteLength - DATA;
  const names = Object.keys(x);
  const fail = (status, message) => {
    const m = new TextEncoder().encode(message);
    bytes.set(m.subarray(0, capacity), DATA);
    ctl[5] = Math.min(m.length, capacity);
    ctl[4] = status;
  };
  for (;;) {
    while (Atomics.load(ctl, 0) !== 1) Atomics.wait(ctl, 0, 2);
    const t0 = performance.now();
    try {
      const fn = x[names[ctl[1]]];
      const want = ctl[2];
      const count = ctl[3];
      const args = [];
      let at = DATA;
      for (let i = 0; i < count; i++) {
        const kind = view.getUint8(at);
        at += 1;
        if (kind === 0) {
          args.push(view.getFloat64(at, true));
          at += 8;
        } else {
          const n = view.getUint32(at, true);
          at += 4;
          // Memory can move as it grows, so it's looked at after the alloc.
          const p = x.gp_alloc(n);
          new Uint8Array(x.memory.buffer, p, n).set(bytes.subarray(at, at + n));
          at += n;
          args.push(p, n);
        }
      }
      const r = fn(...args);
      num[0] = Number(r ?? 0);
      let from = 0;
      let len = 0;
      if (want === 1) {
        from = x.gp_out_ptr();
        len = x.gp_out_len();
      } else if (want === 3) {
        from = r;
        len = 32;
      }
      if (len > capacity) {
        fail(2, `the module's answer is ${len} bytes, more than the page takes (${capacity})`);
      } else {
        bytes.set(new Uint8Array(x.memory.buffer, from, len), DATA);
        ctl[5] = len;
        ctl[4] = 0;
      }
    } catch (e) {
      fail(1, String(e?.message ?? e));
    }
    num[1] = performance.now() - t0;
    Atomics.store(ctl, 0, 2);
  }
};
