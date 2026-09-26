import { defineConfig } from "vite"
import vue from "@vitejs/plugin-vue"
import tailwindcss from "@tailwindcss/vite"

const host = process.env.TAURI_DEV_HOST

export default defineConfig(() => ({
    plugins: [vue(), tailwindcss()],
    clearScreen: false,
    server: {
        port: 1420,
        strictPort: true,
        host: host ?? false,
        hmr: host
            ? {
                  protocol: "ws",
                  host,
                  port: 1421,
              }
            : undefined,
        watch: {
            ignored: ["**/src-tauri/**"],
        },
    },
    build: {
        target: "esnext",
        cssMinify: true,
        sourcemap: false,
        chunkSizeWarningLimit: 1024,
        rolldownOptions: {
            output: {
                legalComments: "none",
                manualChunks(id: string) {
                    if (!id.includes("node_modules")) return
                    if (id.includes("@tiptap") || id.includes("tiptap-")
                        || id.includes("prosemirror")) {
                        return "vendor-editor"
                    }
                    if (id.includes("@tauri-apps")) {
                        return "vendor-tauri"
                    }
                    if (id.includes("vue") || id.includes("@vue")) {
                        return "vendor-vue"
                    }
                    return "vendor"
                },
            },
        },
    },
}))
