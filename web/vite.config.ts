import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

// En production, le reverse proxy sert l'app et l'API sur la même origine ; ici, c'est ce proxy.
const proxy = {
  '/v1': process.env.API_TARGET ?? 'http://127.0.0.1:8080',
  '/health': process.env.API_TARGET ?? 'http://127.0.0.1:8080',
}

export default defineConfig({
  plugins: [react()],
  define: { __APP_VERSION__: JSON.stringify(process.env.npm_package_version ?? '0.1.0') },
  server: { proxy },
  preview: { proxy },
  test: { include: ['src/**/*.test.ts'] },
})
