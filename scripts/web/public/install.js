// Installing a game from a source, and reading what is installed.
//
// Every decision about what a source says is made by gp-registry.wasm, the
// code `gp` checks a registry with (crates/gp-web-registry). This file only
// fetches what that code asks for, and keeps what it accepts.
//
// IndexedDB: database `gp-installs`. Store `blobs` holds files by BLAKE3 hash.
// Store `games` holds a record per install under `NAME@VERSION`, the latest
// version of a name under `latest/NAME`, and the game being played under
// `current`.

const DB = "gp-installs";
const FETCHES_AT_ONCE = 12;

class Unreachable extends Error {}

function request(r) {
  return new Promise((ok, fail) => {
    r.onsuccess = () => ok(r.result);
    r.onerror = () => fail(r.error);
  });
}

function openDb() {
  if (typeof indexedDB === "undefined" || !indexedDB) return Promise.reject(new Error("this browser has no IndexedDB"));
  const r = indexedDB.open(DB, 1);
  r.onupgradeneeded = () => {
    r.result.createObjectStore("blobs");
    r.result.createObjectStore("games");
  };
  return request(r);
}

async function withStore(names, mode, f) {
  const db = await openDb();
  try {
    const tx = db.transaction(names, mode);
    const finished = new Promise((ok, fail) => {
      tx.oncomplete = () => ok();
      tx.onerror = tx.onabort = () => fail(tx.error ?? new Error("the browser stopped the write"));
    });
    const out = await f(tx);
    await finished;
    return out;
  } finally {
    db.close();
  }
}

const enc = new TextEncoder();
const dec = new TextDecoder();

let checker = null;
function registry() {
  checker ??= WebAssembly.instantiateStreaming(fetch(new URL("gp-registry.wasm", import.meta.url)), {}).then(({ instance }) => {
    const x = instance.exports;
    const put = (bytes) => {
      const at = x.gr_alloc(bytes.length);
      new Uint8Array(x.memory.buffer, at, bytes.length).set(bytes);
      return at;
    };
    return {
      resolve(ask) {
        const b = enc.encode(JSON.stringify(ask));
        x.gr_resolve(put(b), b.length);
        return JSON.parse(dec.decode(new Uint8Array(x.memory.buffer, x.gr_out_ptr(), x.gr_out_len())));
      },
      blobOk(hash, bytes) {
        const h = enc.encode(hash);
        return x.gr_blob_ok(put(h), h.length, put(bytes), bytes.length) === 1;
      },
    };
  });
  return checker;
}

// A key of the source: its text or bytes, null when the source doesn't hold
// it. A source that can't be reached is an Unreachable. S3 answers 403 for a
// key that isn't there, because listing is denied.
async function get(source, key, as) {
  let r;
  try {
    r = await fetch(`${source.url.replace(/\/$/, "")}/${key}`, as === "text" ? { cache: "no-store" } : {});
  } catch (err) {
    throw new Unreachable(`${source.url} can't be reached (${err.message ?? err})`);
  }
  if (r.status === 404 || r.status === 403) return null;
  if (!r.ok) throw new Unreachable(`${source.url} answered ${r.status} for ${key}`);
  return as === "text" ? r.text() : new Uint8Array(await r.arrayBuffer());
}

function nowUtc() {
  return new Date().toISOString().replace(/\.\d+Z$/, "Z");
}

async function ask(source, what) {
  const x = await registry();
  const files = {};
  for (;;) {
    const a = x.resolve({ ...what, root: source.root, now: nowUtc(), files });
    if (!a.need) return a;
    if (a.need in files) return { error: `${a.need} was asked for twice` };
    files[a.need] = await get(source, a.need, "text");
  }
}

// The record of an install: NAME@VERSION, or the latest kept of NAME.
export async function installed(name, version = null) {
  return withStore(["games"], "readonly", async (tx) => {
    const s = tx.objectStore("games");
    const v = version ?? (await request(s.get(`latest/${name}`)));
    return v ? ((await request(s.get(`${name}@${v}`))) ?? null) : null;
  });
}

async function haveAll(rec) {
  return withStore(["blobs"], "readonly", async (tx) => {
    const s = tx.objectStore("blobs");
    const keys = new Set(await request(s.getAllKeys()));
    return rec.files.every((f) => keys.has(f.hash));
  });
}

// Asks once for storage that lasts, at the first install. A request not
// granted is asked again at the first save (storage.js).
async function askToKeep() {
  const first = await withStore(["games"], "readonly", async (tx) => (await request(tx.objectStore("games").count())) === 0);
  if (first) await navigator.storage?.persist?.().catch(() => false);
}

async function download(source, r, progress) {
  const x = await registry();
  const missing = await withStore(["blobs"], "readonly", async (tx) => {
    const s = tx.objectStore("blobs");
    const keys = new Set(await request(s.getAllKeys()));
    return [...new Map(r.files.filter((f) => !keys.has(f.hash)).map((f) => [f.hash, f])).values()];
  });
  await askToKeep();
  const got = new Map();
  let next = 0;
  const worker = async () => {
    while (next < missing.length) {
      const f = missing[next++];
      const b = await get(source, `blobs/${f.hash}`);
      if (b === null) throw new Error(`${f.path} isn't on the source`);
      if (b.length !== f.size || !x.blobOk(f.hash, b)) throw new Error(`the file ${f.path} doesn't match what its maker signed`);
      got.set(f.hash, b);
      progress?.(got.size, missing.length);
    }
  };
  await Promise.all(Array.from({ length: FETCHES_AT_ONCE }, worker));
  const rec = { ...r, at: Date.now() };
  // All at once: a record never names files that aren't kept.
  await withStore(["blobs", "games"], "readwrite", async (tx) => {
    const blobs = tx.objectStore("blobs");
    for (const [h, b] of got) blobs.put(b.buffer.slice(b.byteOffset, b.byteOffset + b.length), h);
    const games = tx.objectStore("games");
    games.put(rec, `${r.name}@${r.version}`);
    games.put(r.version, `latest/${r.name}`);
  });
  return rec;
}

// The game or content `what` names, installed. Asks the source each time, so
// new versions and withdrawals are seen; a source that can't be reached or
// doesn't check out leaves what is installed playing, with a `notice`.
// Resolves to { record, notice }.
export async function ensure(source, what, progress) {
  const kept = await installed(what.name, what.version ?? null);
  let a;
  try {
    a = await ask(source, what);
  } catch (err) {
    if (!(err instanceof Unreachable)) throw err;
    if (kept && (await haveAll(kept))) return { record: kept, notice: null, offline: true };
    throw new Error(`it isn't installed in this browser and ${err.message}`);
  }
  if (a.error) {
    if (kept && (await haveAll(kept))) return { record: kept, notice: `Using the copy in this browser: ${a.error}.` };
    throw new Error(a.error);
  }
  const r = a.ok;
  if (r.yank?.severity === "security") throw new Error(`${r.name}@${r.version} was withdrawn for safety: ${r.yank.reason}`);
  const notice = r.yank ? `${r.name}@${r.version} was withdrawn: ${r.yank.reason}.` : null;
  if (kept?.id === r.id && (await haveAll(kept))) return { record: kept, notice };
  return { record: await download(source, r, progress), notice };
}

// What the source offers for a game: [{ name, version, id, title }], the
// latest of each, from its by-game list (gp-registry.wasm's list question).
// A source that can't be reached offers nothing new: the page then plays
// with what it was linked to.
export async function offered(source, game) {
  const a = await ask(source, { name: "", list: true, for_game: game });
  if (a.error) throw new Error(a.error);
  return a.ok;
}

export async function setCurrent(rec) {
  await withStore(["games"], "readwrite", async (tx) => {
    tx.objectStore("games").put(`${rec.name}@${rec.version}`, "current");
  });
}

// A record's files under `folder/` as a Map of path (the folder taken off)
// to ArrayBuffer, each checked against its hash again.
export async function filesOf(rec, folder) {
  const x = await registry();
  const prefix = folder ? `${folder}/` : "";
  const wanted = rec.files.filter((f) => f.path.startsWith(prefix));
  const out = new Map();
  await withStore(["blobs"], "readonly", async (tx) => {
    const s = tx.objectStore("blobs");
    await Promise.all(
      wanted.map(async (f) => {
        const b = await request(s.get(f.hash));
        if (!b) throw new Error(`${f.path} is missing from this browser`);
        if (!x.blobOk(f.hash, new Uint8Array(b))) throw new Error(`${f.path} changed in this browser`);
        out.set(f.path.slice(prefix.length), b);
      }),
    );
  });
  return out;
}

// The current game's play files, or null when nothing is installed.
export async function playFiles() {
  const key = await withStore(["games"], "readonly", async (tx) => request(tx.objectStore("games").get("current")));
  if (!key) return null;
  const at = key.lastIndexOf("@");
  const rec = await installed(key.slice(0, at), key.slice(at + 1));
  return rec ? filesOf(rec, "play") : null;
}
