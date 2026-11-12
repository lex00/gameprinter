// Laya (Convai's open, Jev-compatible decision model) on this machine, over
// Jev's wire format: POST /v1/systemone with {model, state, questions},
// answered with {model, answers, usage}. `gp mechanic demo --decide URL`
// asks it which buttons to hold. The model runs in @receptron/laya (ONNX
// Runtime), installed under $GP_HOME/work/laya by `just laya`; its weights
// are cached under ~/.cache/receptron-laya. It listens on 127.0.0.1 only.
//
// Usage: node scripts/laya/serve.mjs INSTALL_DIR [PORT]

import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const dir = process.argv[2];
const port = Number(process.argv[3] ?? 7720);
if (!dir) {
  console.error("usage: serve.mjs INSTALL_DIR [PORT]");
  process.exit(2);
}
const pkgDir = join(dir, "node_modules", "@receptron", "laya");
const pkg = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));
const entry = pkg.exports?.["."]?.default ?? pkg.main;
const { Laya } = await import(pathToFileURL(join(pkgDir, entry)).href);

const laya = await Laya.load();
const MAX_BODY = 256 * 1024;

const server = createServer((req, res) => {
  // A Content-Length, not chunks: gp's client reads one plain body.
  const reply = (status, body) => {
    const text = Buffer.from(JSON.stringify(body));
    res.writeHead(status, { "Content-Type": "application/json", "Content-Length": text.length, Connection: "close" });
    res.end(text);
  };
  if (req.method !== "POST" || req.url !== "/v1/systemone") return reply(404, { error: "POST /v1/systemone only" });
  let size = 0;
  const chunks = [];
  req.on("data", (c) => {
    size += c.length;
    if (size > MAX_BODY) req.destroy();
    else chunks.push(c);
  });
  req.on("end", async () => {
    let body;
    try {
      body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    } catch {
      return reply(422, { error: "the body is not JSON" });
    }
    if (typeof body?.state !== "object" || typeof body?.questions !== "object") {
      return reply(422, { error: "a request is {model, state, questions}" });
    }
    try {
      const out = await laya.systemOne(body.state, body.questions);
      reply(200, { model: "laya", answers: out.answers, usage: out.usage });
    } catch (e) {
      reply(422, { error: String(e?.message ?? e) });
    }
  });
});
server.listen(port, "127.0.0.1", () => console.log(`laya: http://127.0.0.1:${port}/v1/systemone`));
