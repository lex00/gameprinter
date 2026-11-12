// The inspector's build. `npm run dev` serves it with hot reload and sends
// /api to a running `gp serve` (docs/inspector.md); `npm run build` writes
// editor/dist, which `gp serve` serves itself.
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

declare const process: { env: Record<string, string | undefined> };
const gpServe = process.env.GP_SERVE ?? "http://127.0.0.1:7700";

export default defineConfig({
  plugins: [react()],
  server: {
    host: "127.0.0.1",
    // changeOrigin sends gp serve its own host name, which it requires.
    // Writes (annotations) must also carry its own Origin, so the proxy
    // replaces every request's Origin with gp serve's. That means gp
    // serve's Origin check can't tell pages apart under the dev server:
    // only the JSON Content-Type check (a preflight Vite doesn't grant to
    // other sites) stops another site's writes. Use it for development only.
    proxy: {
      "/api": {
        target: gpServe,
        changeOrigin: true,
        headers: { origin: gpServe },
      },
    },
  },
  build: { outDir: "dist", emptyOutDir: true, sourcemap: false },
  test: {
    include: ["test/**/*.test.ts", "test/**/*.test.tsx"],
    environment: "node",
  },
});
