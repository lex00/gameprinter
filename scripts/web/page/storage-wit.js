// The shell's storage interface, `game-printer:shell@0.1.0`'s `storage`, for a
// JavaScript host: generated from wit/shell/storage.wit by `gp docs
// shell`. Don't edit it; change the WIT and run `just docs`.
//
// A host is an object with each function below, its name in camel case,
// returning a promise: of the WIT's result, with `option` as the value or
// null and `result` as its ok value or a rejection with an Error of its
// error text.

export const WIT_PACKAGE = "game-printer:shell@0.1.0";

/**
 * A file the player handed over.
 * @typedef {object} Picked
 * @property {string} fileName Its name as the player's system gave it.
 * @property {Uint8Array} bytes
 */

/**
 * What a target keeps for one game.
 * @typedef {object} Storage
 * @property {(prefix: string) => Promise<string[]>} list The names kept that start with `prefix` (all of them for ""), sorted.
 * @property {(name: string) => Promise<Uint8Array | null>} read The file's bytes, or none when nothing is kept under the name.
 * @property {(name: string, bytes: Uint8Array) => Promise<void>} write Keeps the bytes under the name, replacing what was there. Either the whole file is kept or none of it, so a crash never leaves half a save.
 * @property {(name: string) => Promise<void>} delete Removes the file. A name that isn't kept is no error.
 * @property {() => Promise<boolean>} persist Asks that what is kept stays when the system wants room (a browser's navigator.storage.persist()). True when it is kept for good; a data folder always is.
 * @property {(name: string, fileName: string) => Promise<string>} export Hands the kept file `name` to the player as a file called `file-name` (a browser downloads it; a native game writes it into its data folder's `exports/`), and says where it went.
 * @property {() => Promise<Picked | null>} import A file the player picks (a browser's file chooser; a native game takes the newest file in its data folder's `imports/`), or none when there is none or the player cancels.
 */

// Each function's name and parameters, in the WIT's order.
export const STORAGE = [
  { name: "list", params: ["prefix"] },
  { name: "read", params: ["name"] },
  { name: "write", params: ["name", "bytes"] },
  { name: "delete", params: ["name"] },
  { name: "persist", params: [] },
  { name: "export", params: ["name", "fileName"] },
  { name: "import", params: [] },
];

// Throws unless `host` has every function of the interface, each taking
// its parameters.
export function checkStorage(host) {
  for (const { name, params } of STORAGE) {
    if (typeof host?.[name] !== "function") throw new Error(`storage host: no ${name}()`);
    if (host[name].length !== params.length) {
      throw new Error(`storage host: ${name}() takes ${host[name].length} parameters, not ${params.length} (${params.join(", ")})`);
    }
  }
  return host;
}
