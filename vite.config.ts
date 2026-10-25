import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const devPort = Number(process.env.VITE_DEV_PORT ?? 1420);

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  test: { environment: "jsdom" },
  server: {
    port: devPort,
    strictPort: true,
    watch: {
      ignored: ["**/target/**", "**/backend/target/**", "**/.git/**", "**/node_modules/**"],
    },
  },
  envPrefix: ["VITE_"],
  build: { target: "es2022" },
});
