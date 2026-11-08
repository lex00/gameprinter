// A small WASI preview1 host with an in-memory file system, enough to run
// gp-web-lift (crates/gp-web-lift) in the browser or in Node: the program's
// files live in a Map, so nothing it reads or writes leaves the page
// (docs/web.md, "The public page").
//
//   const fs = new MemFs();
//   fs.writeFile("/rom.nes", bytes);
//   const { code, stdout, stderr } = await runWasi(module, ["gp-web-lift", "/rom.nes", "/tpl", "/out"], fs);
//   fs.files  // Map of "/out/bundle.json" -> Uint8Array
//
// One directory is preopened, "/" as fd 3. Paths given to the program are
// absolute or relative to it.

const E = { SUCCESS: 0, BADF: 8, EXIST: 20, INVAL: 28, ISDIR: 31, NOENT: 44, NOSYS: 52, NOTDIR: 54 };
const TYPE = { CHAR: 2, DIR: 3, FILE: 4 };
const O = { CREAT: 1, DIRECTORY: 2, EXCL: 4, TRUNC: 8 };

export class MemFs {
  constructor() {
    this.files = new Map(); // absolute path -> Uint8Array
    this.dirs = new Set(["/"]);
  }
  static norm(p) {
    const out = [];
    for (const part of p.split("/")) {
      if (part === "" || part === ".") continue;
      if (part === "..") out.pop();
      else out.push(part);
    }
    return "/" + out.join("/");
  }
  mkdirp(dir) {
    const parts = MemFs.norm(dir).split("/").filter(Boolean);
    let at = "";
    for (const p of parts) {
      at += "/" + p;
      this.dirs.add(at);
    }
  }
  writeFile(path, bytes) {
    const p = MemFs.norm(path);
    this.mkdirp(p.slice(0, p.lastIndexOf("/")) || "/");
    this.files.set(p, bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes));
  }
  readFile(path) {
    return this.files.get(MemFs.norm(path));
  }
  // The names directly inside a directory, sorted, with their types.
  children(dir) {
    const d = dir === "/" ? "/" : dir + "/";
    const seen = new Map();
    for (const [set, type] of [[this.dirs, TYPE.DIR], [this.files.keys(), TYPE.FILE]]) {
      for (const p of set) {
        if (p === dir || !p.startsWith(d)) continue;
        const name = p.slice(d.length);
        if (!name.includes("/")) seen.set(name, type);
      }
    }
    return [...seen.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1));
  }
}

class Exit extends Error {
  constructor(code) {
    super(`exit ${code}`);
    this.code = code;
  }
}

// Runs a WASI command module (a WebAssembly.Module) with `args` over `fs`,
// with no environment. Resolves to its exit code and what it printed.
export async function runWasi(module, args, fs) {
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const out = { 1: [], 2: [] };
  let memory;
  const view = () => new DataView(memory.buffer);
  const bytes = () => new Uint8Array(memory.buffer);
  const str = (ptr, len) => dec.decode(bytes().subarray(ptr, ptr + len));
  // fd -> { dir, path } or { path, pos }
  const fds = new Map([[3, { dir: true, path: "/" }]]);
  let nextFd = 4;
  const resolve = (fd, ptr, len) => {
    const base = fds.get(fd);
    if (!base || !base.dir) return null;
    const rel = str(ptr, len);
    return MemFs.norm(rel.startsWith("/") ? rel : base.path + "/" + rel);
  };
  const filestat = (buf, type, size) => {
    const v = view();
    for (let i = 0; i < 64; i += 8) v.setBigUint64(buf + i, 0n, true);
    v.setUint8(buf + 16, type);
    v.setBigUint64(buf + 24, 1n, true);
    v.setBigUint64(buf + 32, BigInt(size), true);
  };
  const strings = (list, ptrs, buf) => {
    const v = view();
    let at = buf;
    list.forEach((s, i) => {
      const b = enc.encode(s + "\0");
      v.setUint32(ptrs + i * 4, at, true);
      bytes().set(b, at);
      at += b.length;
    });
    return E.SUCCESS;
  };
  const sizes = (list, countPtr, sizePtr) => {
    const v = view();
    v.setUint32(countPtr, list.length, true);
    v.setUint32(sizePtr, list.reduce((n, s) => n + enc.encode(s).length + 1, 0), true);
    return E.SUCCESS;
  };
  const iovs = (ptr, len) => {
    const v = view();
    const list = [];
    for (let i = 0; i < len; i++) list.push([v.getUint32(ptr + i * 8, true), v.getUint32(ptr + i * 8 + 4, true)]);
    return list;
  };
  const wasi = {
    args_sizes_get: (c, s) => sizes(args, c, s),
    args_get: (p, b) => strings(args, p, b),
    environ_sizes_get: (c, s) => sizes([], c, s),
    environ_get: () => E.SUCCESS,
    random_get: (ptr, len) => {
      for (let i = 0; i < len; i += 65536) {
        crypto.getRandomValues(bytes().subarray(ptr + i, ptr + Math.min(len, i + 65536)));
      }
      return E.SUCCESS;
    },
    clock_time_get: (_id, _prec, ptr) => {
      view().setBigUint64(ptr, 0n, true);
      return E.SUCCESS;
    },
    sched_yield: () => E.SUCCESS,
    proc_exit: (code) => {
      throw new Exit(code);
    },
    fd_prestat_get: (fd, buf) => {
      if (fd !== 3) return E.BADF;
      view().setUint8(buf, 0);
      view().setUint32(buf + 4, 1, true);
      return E.SUCCESS;
    },
    fd_prestat_dir_name: (fd, ptr, len) => {
      if (fd !== 3) return E.BADF;
      bytes().set(enc.encode("/").subarray(0, len), ptr);
      return E.SUCCESS;
    },
    fd_fdstat_get: (fd, buf) => {
      const f = fds.get(fd);
      if (fd > 2 && !f) return E.BADF;
      const v = view();
      v.setUint8(buf, fd <= 2 ? TYPE.CHAR : f.dir ? TYPE.DIR : TYPE.FILE);
      v.setUint16(buf + 2, 0, true);
      v.setBigUint64(buf + 8, 0xffffffffffffffffn, true);
      v.setBigUint64(buf + 16, 0xffffffffffffffffn, true);
      return E.SUCCESS;
    },
    fd_filestat_get: (fd, buf) => {
      const f = fds.get(fd);
      if (fd <= 2) return filestat(buf, TYPE.CHAR, 0), E.SUCCESS;
      if (!f) return E.BADF;
      filestat(buf, f.dir ? TYPE.DIR : TYPE.FILE, f.dir ? 0 : fs.files.get(f.path).length);
      return E.SUCCESS;
    },
    path_filestat_get: (fd, _flags, ptr, len, buf) => {
      const p = resolve(fd, ptr, len);
      if (p === null) return E.BADF;
      if (fs.dirs.has(p)) return filestat(buf, TYPE.DIR, 0), E.SUCCESS;
      const f = fs.files.get(p);
      if (!f) return E.NOENT;
      filestat(buf, TYPE.FILE, f.length);
      return E.SUCCESS;
    },
    path_create_directory: (fd, ptr, len) => {
      const p = resolve(fd, ptr, len);
      if (p === null) return E.BADF;
      if (fs.dirs.has(p) || fs.files.has(p)) return E.EXIST;
      const parent = p.slice(0, p.lastIndexOf("/")) || "/";
      if (!fs.dirs.has(parent)) return E.NOENT;
      fs.dirs.add(p);
      return E.SUCCESS;
    },
    path_open: (fd, _dirflags, ptr, len, oflags, _rb, _ri, _fdflags, out) => {
      const p = resolve(fd, ptr, len);
      if (p === null) return E.BADF;
      if (fs.dirs.has(p)) {
        if (oflags & (O.CREAT | O.TRUNC) && !(oflags & O.DIRECTORY)) return E.ISDIR;
        fds.set(nextFd, { dir: true, path: p });
      } else {
        if (oflags & O.DIRECTORY) return fs.files.has(p) ? E.NOTDIR : E.NOENT;
        const exists = fs.files.has(p);
        if (exists && oflags & O.EXCL) return E.EXIST;
        if (!exists && !(oflags & O.CREAT)) return E.NOENT;
        const parent = p.slice(0, p.lastIndexOf("/")) || "/";
        if (!fs.dirs.has(parent)) return E.NOENT;
        if (!exists || oflags & O.TRUNC) fs.files.set(p, new Uint8Array(0));
        fds.set(nextFd, { path: p, pos: 0 });
      }
      view().setUint32(out, nextFd++, true);
      return E.SUCCESS;
    },
    fd_close: (fd) => (fds.delete(fd) ? E.SUCCESS : fd <= 2 ? E.SUCCESS : E.BADF),
    fd_read: (fd, ptr, len, nread) => {
      const f = fds.get(fd);
      if (fd === 0) return view().setUint32(nread, 0, true), E.SUCCESS;
      if (!f || f.dir) return E.BADF;
      const data = fs.files.get(f.path);
      let n = 0;
      for (const [buf, l] of iovs(ptr, len)) {
        const chunk = data.subarray(f.pos, f.pos + l);
        bytes().set(chunk, buf);
        f.pos += chunk.length;
        n += chunk.length;
        if (chunk.length < l) break;
      }
      view().setUint32(nread, n, true);
      return E.SUCCESS;
    },
    fd_write: (fd, ptr, len, nwritten) => {
      let n = 0;
      const parts = iovs(ptr, len).map(([b, l]) => bytes().slice(b, b + l));
      for (const p of parts) n += p.length;
      if (fd === 1 || fd === 2) {
        out[fd].push(...parts);
      } else {
        const f = fds.get(fd);
        if (!f || f.dir) return E.BADF;
        let data = fs.files.get(f.path);
        const end = f.pos + n;
        if (end > data.length) {
          // The file is a view of its true length on a buffer that grows
          // by doubling, so many small writes stay cheap.
          if (data.byteOffset === 0 && data.buffer.byteLength >= end) {
            data = new Uint8Array(data.buffer, 0, end);
          } else {
            const grown = new Uint8Array(Math.max(end, data.length * 2, 4096));
            grown.set(data);
            data = grown.subarray(0, end);
          }
        }
        for (const p of parts) {
          data.set(p, f.pos);
          f.pos += p.length;
        }
        fs.files.set(f.path, data);
      }
      view().setUint32(nwritten, n, true);
      return E.SUCCESS;
    },
    fd_seek: (fd, offset, whence, out) => {
      const f = fds.get(fd);
      if (!f || f.dir) return E.BADF;
      const size = fs.files.get(f.path).length;
      const base = whence === 0 ? 0 : whence === 1 ? f.pos : size;
      const pos = base + Number(offset);
      if (pos < 0) return E.INVAL;
      f.pos = pos;
      view().setBigUint64(out, BigInt(pos), true);
      return E.SUCCESS;
    },
    fd_tell: (fd, out) => {
      const f = fds.get(fd);
      if (!f || f.dir) return E.BADF;
      view().setBigUint64(out, BigInt(f.pos), true);
      return E.SUCCESS;
    },
    fd_readdir: (fd, buf, len, cookie, used) => {
      const f = fds.get(fd);
      if (!f || !f.dir) return E.BADF;
      const list = fs.children(f.path);
      const v = view();
      let at = 0;
      for (let i = Number(cookie); i < list.length && at < len; i++) {
        const [name, type] = list[i];
        const nb = enc.encode(name);
        const ent = new Uint8Array(24 + nb.length);
        const ev = new DataView(ent.buffer);
        ev.setBigUint64(0, BigInt(i + 1), true);
        ev.setBigUint64(8, BigInt(i + 1), true);
        ev.setUint32(16, nb.length, true);
        ev.setUint8(20, type);
        ent.set(nb, 24);
        const take = Math.min(ent.length, len - at);
        bytes().set(ent.subarray(0, take), buf + at);
        at += take;
      }
      v.setUint32(used, at, true);
      return E.SUCCESS;
    },
  };
  // Anything else the module imports answers "not supported".
  const imports = { wasi_snapshot_preview1: new Proxy(wasi, { get: (t, k) => t[k] ?? (() => E.NOSYS) }) };
  const instance = await WebAssembly.instantiate(module, imports);
  memory = instance.exports.memory;
  let code = 0;
  try {
    instance.exports._start();
  } catch (e) {
    if (e instanceof Exit) code = e.code;
    else throw e;
  }
  const text = (parts) => dec.decode(concat(parts));
  return { code, stdout: text(out[1]), stderr: text(out[2]) };
}

function concat(parts) {
  const n = parts.reduce((a, p) => a + p.length, 0);
  const b = new Uint8Array(n);
  let at = 0;
  for (const p of parts) {
    b.set(p, at);
    at += p.length;
  }
  return b;
}
