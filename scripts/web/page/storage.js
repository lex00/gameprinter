// The runtime shell's storage in the browser (docs/shell.md, "Storage"):
// the interface of wit/shell/storage.wit, whose generated check is
// storage-wit.js, kept in IndexedDB. The generated game keeps the same
// names in its data folder.
//
// Each game's files are in the database `gp-saves`, store `files`, under
// `<game>/<name>` (the game is preview.json's `model`), as bytes. Where
// IndexedDB can't be opened (a browser that blocks it), localStorage keeps
// them as base64 under `gp-file:<game>/<name>`, as long as it has room.

import { checkStorage } from "./storage-wit.js";

const DB = "gp-saves";
const STORE = "files";

const done = (req) =>
  new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error("IndexedDB request failed"));
  });

function openDb() {
  if (typeof indexedDB === "undefined") return Promise.reject(new Error("no IndexedDB"));
  const req = indexedDB.open(DB, 1);
  req.onupgradeneeded = () => req.result.createObjectStore(STORE);
  return done(req);
}

const b64 = (bytes) => btoa(Array.from(bytes, (c) => String.fromCharCode(c)).join(""));
const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

// Hands bytes to the player as a file: the browser downloads it.
function download(bytes, fileName) {
  const url = URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
  return `downloaded as ${fileName}`;
}

// A file the player picks with the browser's file chooser, or null when
// they cancel. A browser opens the chooser only while a click or a key is
// handled.
function pick() {
  return new Promise((resolve, reject) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".gpsave";
    input.addEventListener("cancel", () => resolve(null));
    input.addEventListener("change", () => {
      const f = input.files?.[0];
      if (!f) return resolve(null);
      f.arrayBuffer().then((b) => resolve({ fileName: f.name, bytes: new Uint8Array(b) }), reject);
    });
    input.click();
  });
}

async function persist() {
  return (await navigator.storage?.persist?.()) ?? false;
}

function idbHost(db, game) {
  const key = (name) => `${game}/${name}`;
  const tx = (mode) => db.transaction(STORE, mode).objectStore(STORE);
  // Resolves once the transaction is written whole.
  const commit = (fn) =>
    new Promise((resolve, reject) => {
      const t = db.transaction(STORE, "readwrite");
      fn(t.objectStore(STORE));
      t.oncomplete = () => resolve();
      t.onerror = t.onabort = () => reject(t.error ?? new Error("IndexedDB write failed"));
    });
  const host = {
    async list(prefix) {
      const from = key(prefix);
      const keys = await done(tx("readonly").getAllKeys(IDBKeyRange.bound(from, `${from}￿`)));
      return keys.map((k) => String(k).slice(game.length + 1)).sort();
    },
    async read(name) {
      const v = await done(tx("readonly").get(key(name)));
      return v == null ? null : new Uint8Array(v);
    },
    write(name, bytes) {
      return commit((s) => s.put(new Uint8Array(bytes), key(name)));
    },
    delete(name) {
      return commit((s) => s.delete(key(name)));
    },
    persist() {
      return persist();
    },
    async export(name, fileName) {
      const bytes = await host.read(name);
      if (!bytes) throw new Error(`nothing is kept as ${name}`);
      return download(bytes, fileName);
    },
    import() {
      return pick();
    },
  };
  return host;
}

function localHost(game) {
  const key = (name) => `gp-file:${game}/${name}`;
  const ls = () => window.localStorage;
  const host = {
    async list(prefix) {
      const from = key(prefix);
      const out = [];
      for (let i = 0; i < ls().length; i++) {
        const k = ls().key(i);
        if (k?.startsWith(from)) out.push(k.slice(key("").length));
      }
      return out.sort();
    },
    async read(name) {
      const v = ls().getItem(key(name));
      return v == null ? null : unb64(v);
    },
    async write(name, bytes) {
      ls().setItem(key(name), b64(bytes));
    },
    async delete(name) {
      ls().removeItem(key(name));
    },
    persist() {
      return persist();
    },
    async export(name, fileName) {
      const bytes = await host.read(name);
      if (!bytes) throw new Error(`nothing is kept as ${name}`);
      return download(bytes, fileName);
    },
    import() {
      return pick();
    },
  };
  return host;
}

// The storage of one game: IndexedDB, else localStorage. Checked against
// the WIT's functions (checkStorage) before it is handed out.
export async function openStorage(game) {
  const db = await openDb().catch(() => null);
  return checkStorage(db ? idbHost(db, game) : localHost(game));
}
