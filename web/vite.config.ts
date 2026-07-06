import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
// The control-plane base for this run (default :7878; the e2e harness shifts it per run for
// concurrency-safety — see e2e/ports.mjs). The browser opens streams same-origin and the
// preview/dev server proxies them to this backend, so the proxy target must track the CP port.
import { aliceCP, ports } from "./e2e/ports.mjs";

// The workbench is a client-owned Solid island served by Vite; Tauri wraps this
// same build for desktop (`app-stack.md`). The control plane runs separately on
// loopback (default :7878) — the dev/preview server proxies stream calls to it.
const proxy = {
    "/scopes": aliceCP,
    "/engagements": aliceCP,
};

export default defineConfig({
    plugins: [solid()],
    build: {
        // Multi-page: the workbench (index.html) plus the embed demo page
        // (embed-example.html) that mounts the EMBED-2 custom elements.
        rollupOptions: {
            input: {
                main: "index.html",
                "embed-example": "embed-example.html",
            },
        },
    },
    server: {
        // Pin the dev port. The control plane's CORS allowlist only blesses the
        // canonical web origins (5173 dev, 4173 preview) — so Vite's default
        // behaviour of silently auto-incrementing to 5174 when 5173 is busy lands
        // the app on an origin the backend rejects ("Failed to fetch", with nothing
        // obviously wrong). `strictPort` makes a port clash a loud, immediate
        // failure instead, so you fix the real problem (a stray dev server) rather
        // than chase a phantom CORS bug.
        port: 5173,
        strictPort: true,
        proxy,
    },
    // The e2e harness drives `vite preview`; its port (passed via --port) and proxy track
    // this run's resolved ports so concurrent runs don't collide.
    preview: {
        port: ports.preview,
        strictPort: true,
        proxy,
    },
});
