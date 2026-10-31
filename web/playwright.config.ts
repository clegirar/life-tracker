import { defineConfig } from '@playwright/test'

// Lancé par web/test.sh, qui démarre le vrai backend et le vrai PostgreSQL et fournit E2E_*.
const port = process.env.E2E_WEB_PORT ?? '4173'

export default defineConfig({
  testDir: './e2e',
  outputDir: './e2e-artifacts/results',
  // Une seule base, et une seule journée « aujourd'hui » : les tests ne se croisent pas.
  workers: 1,
  fullyParallel: false,
  reporter: 'list',
  timeout: 30_000,
  use: {
    // localhost : le navigateur accepte le cookie Secure, le même qu'en production.
    baseURL: `http://localhost:${port}`,
    browserName: 'chromium',
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
    locale: 'fr-FR',
    trace: 'retain-on-failure',
  },
  webServer: {
    // Le bundle de production ; E2E_VITE_DEV=1 essaie le serveur de développement.
    command: `npx vite ${process.env.E2E_VITE_DEV ? '' : 'preview'} --port ${port} --strictPort`,
    url: `http://localhost:${port}/manifest.webmanifest`,
    reuseExistingServer: false,
  },
})
