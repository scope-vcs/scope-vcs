import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import tailwindcss from '@tailwindcss/vite'
import viteReact from '@vitejs/plugin-react'
import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { nitro } from 'nitro/vite'
import { defineConfig, loadEnv, searchForWorkspaceRoot, type Plugin } from 'vite'

const legalDirectory = path.resolve(import.meta.dirname, '../legal')
const clientChunkGraphPath = path.resolve(import.meta.dirname, '.output/client-chunk-graph.json')

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, import.meta.dirname, '')
  const allowedHost = env.SCOPE_WEB_ALLOWED_HOST?.trim()

  return {
    server: {
      allowedHosts: allowedHost ? [allowedHost] : [],
      fs: { allow: [searchForWorkspaceRoot(import.meta.dirname), legalDirectory] },
      port: 3000,
    },
    resolve: {
      alias: {
        '@': path.resolve(import.meta.dirname, './src'),
        '@legal': legalDirectory,
      },
      dedupe: ['react', 'react-dom'],
    },
    plugins: [
      tailwindcss(),
      tanstackStart({
        srcDirectory: 'src',
        server: {
          build: {
            inlineCss: false,
          },
        },
      }),
      viteReact(),
      clientChunkGraph(),
      nitro({
        compressPublicAssets: {
          brotli: true,
          gzip: true,
        },
        plugins: [
          './src/server/readiness-endpoint.ts',
          './src/server/analytics-endpoint.ts',
          './src/server/compress-responses.ts',
          './src/server/secure-responses.ts',
        ],
      }),
    ],
  }
})

function clientChunkGraph(): Plugin {
  return {
    name: 'scope:client-chunk-graph',
    applyToEnvironment: (environment) => environment.name === 'client',
    generateBundle(_options, bundle) {
      const chunks = Object.values(bundle).flatMap((output) => output.type === 'chunk'
        ? [{
          fileName: output.fileName,
          isEntry: output.isEntry,
          imports: output.imports,
          dynamicImports: output.dynamicImports,
        }]
        : [])
      mkdirSync(path.dirname(clientChunkGraphPath), { recursive: true })
      writeFileSync(clientChunkGraphPath, JSON.stringify(chunks, null, 2))
    },
  }
}
