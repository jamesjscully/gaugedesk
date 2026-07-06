import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { fileURLToPath } from "node:url";

const webRoot = fileURLToPath(new URL("../../", import.meta.url));

export default defineConfig({
    root: webRoot,
    plugins: [solid()],
    build: {
        outDir: "dist-app-mobile-web",
        emptyOutDir: true,
        rollupOptions: {
            input: {
                mobile: fileURLToPath(new URL("index.html", import.meta.url)),
            },
        },
    },
});
