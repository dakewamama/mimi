import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The agent's origin. Both the dev server and `vite preview` need the proxy:
// `server.proxy` does NOT apply to preview, so a built bundle served locally
// had no /api route at all and every fetch failed.
//
// Use 127.0.0.1, not localhost: on WSL2/Windows, "localhost" often resolves to
// the IPv6 loopback (::1) first, and axum's default bind is IPv4-only. The
// proxy would then dial a socket nothing is listening on -- which surfaces in
// the browser as ERR_SOCKET_NOT_CONNECTED, not a clean connection-refused.
const AGENT = process.env.MIMI_AGENT_URL || "http://127.0.0.1:8080";

const proxy = {
  "/api": {
    target: AGENT,
    changeOrigin: true,
    rewrite: (p) => p.replace(/^\/api/, ""),
  },
};

export default defineConfig({
  plugins: [react()],
  server: { port: 5173, host: true, proxy },
  preview: { port: 4173, host: true, proxy },
});