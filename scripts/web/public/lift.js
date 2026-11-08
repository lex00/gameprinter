// The public page's ROM lift (docs/web.md, "The public page"): a visitor's
// own ROM is hash-checked and lifted in their browser by gp-web-lift.wasm
// (crates/gp-web-lift) over an in-memory file system (wasi.js), and what
// the preview loads is kept in this browser's IndexedDB only. The ROM's
// bytes go to the module and nowhere else: this file makes no request
// with them, only a GET of the module.
//
//   const r = await liftRom(file, { moduleUrl: "lift/gp-web-lift.wasm", templateFiles });
//   r.ok ? (await loadStored("donkey-kong")) : r.error
//
// IndexedDB: database `gp-public`, store `files`, keys `GAME/<path>`
// (`donkey-kong/bundle.json`, `donkey-kong/assets/present.txt`), values
// ArrayBuffers, `GAME/.rom` holding the ROM itself, and `GAME/.ok` holding
// { sha256, files, found, at }, where `found` is what the lift found and
// filled in (null for a one-template folder). The ROM is kept so a later
// visit lifts it again with that visit's page (storedRom): files lifted
// by an older page lack what the page added since, such as its themes.

import { MemFs, runWasi } from "./wasi.js";

const DB = "gp-public";
const STORE = "files";

// IndexedDB can be missing, refused (a private window in some browsers)
// or, in some Safari versions, never answer an open; each ends in an
// error in words, the last after OPEN_WAIT_MS.
const OPEN_WAIT_MS = 10000;
const NO_STORAGE =
  "this browser didn't let the page keep files (IndexedDB), which the game reads the ROM's pieces from. " +
  "A private window can refuse it; try a normal window";

function openDb() {
  return new Promise((ok, fail) => {
    let req;
    try {
      if (typeof indexedDB === "undefined" || !indexedDB) throw new Error("no IndexedDB");
      req = indexedDB.open(DB, 1);
    } catch (err) {
      fail(new Error(`${NO_STORAGE} (${err?.message ?? err})`));
      return;
    }
    const wait = setTimeout(() => fail(new Error(`${NO_STORAGE} (it didn't answer)`)), OPEN_WAIT_MS);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => {
      clearTimeout(wait);
      ok(req.result);
    };
    req.onerror = () => {
      clearTimeout(wait);
      fail(new Error(`${NO_STORAGE} (${req.error?.message ?? req.error})`));
    };
    req.onblocked = () => {
      clearTimeout(wait);
      fail(new Error("the page's storage is in use by another tab of this page with an older version; close it and try again"));
    };
  });
}

function done(tx) {
  return new Promise((ok, fail) => {
    tx.oncomplete = () => ok();
    tx.onerror = tx.onabort = () => fail(tx.error ?? new Error("the browser stopped the write"));
  });
}

// A File's bytes: Blob.arrayBuffer where the browser has it, else a
// FileReader (Safari before 14). A file the browser can't read (moved or
// deleted since it was picked, or refused by its sandbox) is an error in
// words.
async function toBytes(file) {
  if (file instanceof Uint8Array) return file;
  if (file instanceof ArrayBuffer) return new Uint8Array(file);
  const unread = (err) => new Error(`the browser couldn't read the file (${err?.message ?? err ?? "no reason given"}); pick it again`);
  if (typeof file?.arrayBuffer === "function") {
    try {
      return new Uint8Array(await file.arrayBuffer());
    } catch (err) {
      if (typeof FileReader === "undefined") throw unread(err);
    }
  }
  return new Promise((ok, fail) => {
    const r = new FileReader();
    r.onload = () => ok(new Uint8Array(r.result));
    r.onerror = () => fail(unread(r.error));
    r.readAsArrayBuffer(file);
  });
}

// The module, compiled while it downloads where the browser can; else (no
// compileStreaming, or a server that sends it as another type) from its
// bytes.
async function compileModule(url) {
  if (typeof WebAssembly === "undefined") throw new Error("this browser has no WebAssembly, which reads the ROM");
  if (typeof WebAssembly.compileStreaming === "function") {
    try {
      return await WebAssembly.compileStreaming(fetch(url));
    } catch {
      // Compiled from the bytes below, which says what went wrong.
    }
  }
  const r = await fetch(url);
  if (!r.ok) throw new Error(`the ROM reader's module didn't load (${r.status})`);
  return WebAssembly.compile(await r.arrayBuffer());
}

// Lifts `file` (a File, Blob, ArrayBuffer or Uint8Array). `templateFiles`
// is the installed game's play files under lift/: index.json
// (`{ game, files: [...], alias: { path: url } }`) and those files.
// Resolves to gp-web-lift's result.json: { ok: true, sha256, files, found,
// looks_skipped } once stored, where looks_skipped lists each theme the
// lift couldn't offer as { id, title, reason, detail }, or { ok: false,
// error, expected, got }.
export async function liftRom(file, { moduleUrl, templateFiles, extraThemes = [] }) {
  const read = (name) => {
    const b = templateFiles.get(name);
    if (!b) throw new Error(`the installed game has no lift/${name}`);
    return new Uint8Array(b);
  };
  const index = JSON.parse(new TextDecoder().decode(read("index.json")));
  const game = index.game;
  const fs = new MemFs();
  const bytes = await toBytes(file);
  fs.writeFile("/rom.nes", bytes);
  // `alias` maps a path the lift reads to one beside the lift folder.
  const here = (from) => new URL(from, "http://x/lift/").pathname.slice(1);
  const put = new Map(index.files.map((n) => [n, read(n)]));
  for (const [name, from] of Object.entries(index.alias ?? {})) {
    const b = templateFiles.get(here(from));
    if (!b) throw new Error(`the installed game has no ${here(from)}`);
    put.set(name, new Uint8Array(b));
  }
  // A theme from another package, offered beside the game's own: its files
  // in the first template, and a pointer to them in each ROM it loads.
  const { templates } = JSON.parse(new TextDecoder().decode(read("game.json")));
  const owner = templates.find((t) => !put.has(`${t}/load.json`));
  for (const { id, files } of extraThemes) {
    for (const t of templates) {
      for (const k of [...put.keys()]) if (k.startsWith(`${t}/themes/${id}/`)) put.delete(k);
      if (t === owner) for (const [n, b] of files) put.set(`${t}/themes/${id}/${n}`, new Uint8Array(b));
      else put.set(`${t}/themes/${id}/same_as.toml`, new TextEncoder().encode(`theme = "../../../${owner}/themes/${id}"\n`));
    }
  }
  for (const [name, b] of put) fs.writeFile(`/template/${name}`, b);
  fs.mkdirp("/out");
  const module = await compileModule(new URL(moduleUrl, document.baseURI));
  const t0 = performance.now();
  const run = await runWasi(module, ["gp-web-lift", "/rom.nes", "/template", "/out"], fs);
  const raw = fs.readFile("/out/result.json");
  const result = raw
    ? JSON.parse(new TextDecoder().decode(raw))
    : { ok: false, error: run.stderr.trim() || `the lift stopped (exit ${run.code})` };
  result.ms = Math.round(performance.now() - t0);
  if (!result.ok) return result;
  await clearStored(game);
  const db = await openDb();
  try {
    const tx = db.transaction(STORE, "readwrite");
    const store = tx.objectStore(STORE);
    for (const name of result.files) {
      store.put(fs.readFile(`/out/${name}`).slice().buffer, `${game}/${name}`);
    }
    store.put(bytes.slice().buffer, `${game}/.rom`);
    store.put({ sha256: result.sha256, files: result.files, found: result.found ?? null, at: Date.now() }, `${game}/.ok`);
    await done(tx);
  } catch (err) {
    // Full storage, or a private window that opens the database but keeps
    // nothing in it.
    throw new Error(`${NO_STORAGE} (${err?.message ?? err})`);
  } finally {
    db.close();
  }
  return result;
}

// The stored files of `game` as a Map of path -> ArrayBuffer, or null when
// nothing complete is stored.
export async function loadStored(game) {
  const db = await openDb();
  const tx = db.transaction(STORE, "readonly");
  const store = tx.objectStore(STORE);
  const get = (k) =>
    new Promise((ok, fail) => {
      const r = store.get(k);
      r.onsuccess = () => ok(r.result);
      r.onerror = () => fail(r.error);
    });
  const meta = await get(`${game}/.ok`);
  if (!meta) {
    db.close();
    return null;
  }
  const files = new Map();
  for (const name of meta.files) {
    const v = await get(`${game}/${name}`);
    if (!v) {
      db.close();
      return null;
    }
    files.set(name, v);
  }
  db.close();
  return files;
}

// What was stored with `game`'s files: { sha256, files, found, at }, or
// null.
export async function storedMeta(game) {
  const db = await openDb();
  const tx = db.transaction(STORE, "readonly");
  const meta = await new Promise((ok, fail) => {
    const r = tx.objectStore(STORE).get(`${game}/.ok`);
    r.onsuccess = () => ok(r.result ?? null);
    r.onerror = () => fail(r.error);
  });
  db.close();
  return meta;
}

// The ROM stored with `game`'s files, as an ArrayBuffer, or null (nothing
// stored, or files kept by a page from before 2026-11-04 that kept no ROM).
export async function storedRom(game) {
  const db = await openDb();
  const tx = db.transaction(STORE, "readonly");
  const rom = await new Promise((ok, fail) => {
    const r = tx.objectStore(STORE).get(`${game}/.rom`);
    r.onsuccess = () => ok(r.result ?? null);
    r.onerror = () => fail(r.error);
  });
  db.close();
  return rom;
}

// Deletes every stored file of `game`, its ROM included.
export async function clearStored(game) {
  const db = await openDb();
  const tx = db.transaction(STORE, "readwrite");
  tx.objectStore(STORE).delete(IDBKeyRange.bound(`${game}/`, `${game}/\uffff`));
  await done(tx);
  db.close();
}
