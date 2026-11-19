import { fileURLToPath } from 'node:url'
import { build } from 'vite'

const webRoot = fileURLToPath(new URL('..', import.meta.url))
const workerEntry = fileURLToPath(new URL(
  '../src/features/code-render/code-render-worker.ts',
  import.meta.url,
))
const workerOutput = fileURLToPath(new URL(
  '../.output/server/_workers',
  import.meta.url,
))

await build({
  build: {
    copyPublicDir: false,
    emptyOutDir: true,
    minify: true,
    outDir: workerOutput,
    rollupOptions: {
      output: {
        chunkFileNames: 'chunks/[name]-[hash].mjs',
        entryFileNames: 'code-render-worker.mjs',
      },
    },
    ssr: workerEntry,
    target: 'node24',
  },
  configFile: false,
  logLevel: 'warn',
  root: webRoot,
  ssr: {
    noExternal: true,
  },
})
