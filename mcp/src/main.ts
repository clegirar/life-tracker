// Point d'entrée : `serve` (par défaut) et `hash-password` (le secret de connexion, lu sur l'entrée
// standard, jamais en argument).

import { createInterface } from 'node:readline';

import { createApp } from './app.js';
import { Backend } from './backend.js';
import { loadConfig } from './config.js';
import { hashSecret } from './secret.js';
import { TokenStore } from './store.js';

/** Journal JSON sur une ligne : des événements et des statuts, jamais de jeton, de code ni d'adresse. */
function log(entry: Record<string, unknown>): void {
  process.stdout.write(`${JSON.stringify({ time: new Date().toISOString(), level: 'info', ...entry })}\n`);
}

async function firstLine(): Promise<string> {
  const lines = createInterface({ input: process.stdin, terminal: false });
  for await (const line of lines) {
    lines.close();
    return line;
  }
  return '';
}

async function hashPassword(): Promise<void> {
  if (process.stdin.isTTY) {
    process.stderr.write('Le secret se lit sur l’entrée standard : printf \'%s\\n\' "$SECRET" | … hash-password\n');
    process.exit(2);
  }
  const secret = (await firstLine()).replace(/\r$/, '');
  const hash = await hashSecret(secret);
  // Encodé : un `$` dans un fichier d'environnement serait interpolé par Compose.
  process.stdout.write(`MCP_LOGIN_HASH=${Buffer.from(hash, 'utf8').toString('base64url')}\n`);
}

function serve(): void {
  const config = loadConfig();
  const app = createApp(config, {
    now: Date.now,
    store: new TokenStore(config.stateDir, Date.now),
    backend: new Backend(config.backendUrl, config.serviceToken),
    log,
  });
  const server = app.listen(config.listenPort, config.listenHost, () => {
    log({ msg: 'listening', port: config.listenPort });
  });
  const stop = () => {
    server.close(() => {
      log({ msg: 'stopped' });
      process.exit(0);
    });
    setTimeout(() => process.exit(0), 5000).unref();
  };
  process.on('SIGTERM', stop);
  process.on('SIGINT', stop);
}

const command = process.argv[2] ?? 'serve';
if (command === 'serve') {
  serve();
} else if (command === 'hash-password') {
  hashPassword().catch((error: unknown) => {
    process.stderr.write(`${error instanceof Error ? error.message : 'échec'}\n`);
    process.exit(1);
  });
} else {
  process.stderr.write('commandes : serve, hash-password\n');
  process.exit(2);
}
