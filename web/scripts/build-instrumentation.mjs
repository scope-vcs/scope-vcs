import { fileURLToPath } from 'node:url'
import { build } from 'vite'

const webRoot = fileURLToPath(new URL('..', import.meta.url))
const instrumentationEntry = fileURLToPath(new URL(
  '../src/server/instrumentation.ts',
  import.meta.url,
))
const instrumentationOutput = fileURLToPath(new URL(
  '../.output/server/_instrumentation',
  import.meta.url,
))

await build({
  build: {
    copyPublicDir: false,
    emptyOutDir: true,
    minify: false,
    outDir: instrumentationOutput,
    rollupOptions: {
      output: {
        entryFileNames: 'instrumentation.mjs',
        codeSplitting: false,
      },
    },
    ssr: instrumentationEntry,
    target: 'node24',
  },
  configFile: false,
  logLevel: 'warn',
  root: webRoot,
  ssr: {
    noExternal: true,
  },
})
