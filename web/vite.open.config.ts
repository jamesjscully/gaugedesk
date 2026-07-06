import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Open-source workbench build lane (ADR 0069): the canonical workbench
// app entry (`apps/workbench-web/src/main.tsx`, via the root `index.html`)
// compiles with the open settings menu from `@gaugewright/workbench-ui`,
// excluding enterprise admin, settlement, and managed embed-host panels from
// the bundle.
export default defineConfig({
    plugins: [solid()],
    build: {
        outDir: "dist-open",
        emptyOutDir: true,
        rollupOptions: {
            input: {
                main: "index.html",
            },
        },
    },
});
