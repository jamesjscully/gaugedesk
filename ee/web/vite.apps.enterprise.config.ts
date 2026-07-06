import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { fileURLToPath } from "node:url";

const webRoot = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig({
    root: webRoot,
    plugins: [solid()],
    build: {
        outDir: "dist-apps-enterprise",
        emptyOutDir: true,
        rollupOptions: {
            input: {
                admin: fileURLToPath(
                    new URL("apps/admin-console/index.html", import.meta.url),
                ),
            },
        },
    },
});
