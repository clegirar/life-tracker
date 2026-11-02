// Outils des tests : un serveur MCP en mémoire sur un port libre, une horloge réglable, le flux OAuth.
// L'origine publique est https://localhost:8443 : l'en-tête Host de fetch (localhost:<port>) passe la
// protection contre le DNS rebinding, comme le nom public en production.

import { mkdtempSync, rmSync } from 'node:fs';
import { request } from 'node:http';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { CLIENT_ID, createApp, REDIRECT_URIS, s256 } from '../src/app.js';
import { Backend } from '../src/backend.js';
import { type Config } from '../src/config.js';
import { hashSecret } from '../src/secret.js';
import { randomToken, TokenStore } from '../src/store.js';

export const SECRET = 'un secret de test assez long';
export const PUBLIC_ORIGIN = 'https://localhost:8443';
export const RESOURCE = `${PUBLIC_ORIGIN}/mcp`;
export const REDIRECT = REDIRECT_URIS[0]!;

let loginHash: string | undefined;

export interface Harness {
  base: string;
  port: number;
  store: TokenStore;
  clock: { now: number };
  logs: Record<string, unknown>[];
  stateDir: string;
  close: () => Promise<void>;
}

export async function startServer(options: { backendUrl?: string; serviceToken?: string } = {}): Promise<Harness> {
  loginHash ??= await hashSecret(SECRET);
  const stateDir = mkdtempSync(join(tmpdir(), 'lt-mcp-test-'));
  const clock = { now: Date.now() };
  const now = () => clock.now;
  const config: Config = {
    publicOrigin: PUBLIC_ORIGIN,
    resource: RESOURCE,
    publicHostname: 'localhost',
    backendUrl: options.backendUrl ?? 'http://127.0.0.1:9',
    serviceToken: options.serviceToken ?? 'jeton-de-service-factice',
    loginHash,
    listenHost: '127.0.0.1',
    listenPort: 0,
    stateDir,
  };
  const store = new TokenStore(stateDir, now);
  const logs: Record<string, unknown>[] = [];
  const app = createApp(config, {
    now,
    store,
    backend: new Backend(config.backendUrl, config.serviceToken),
    log: (entry) => logs.push(entry),
  });
  const server = app.listen(0, '127.0.0.1');
  await new Promise<void>((resolve) => server.once('listening', () => resolve()));
  const port = (server.address() as AddressInfo).port;
  return {
    base: `http://localhost:${port}`,
    port,
    store,
    clock,
    logs,
    stateDir,
    close: () =>
      new Promise<void>((resolve) => {
        server.closeAllConnections();
        server.close(() => {
          rmSync(stateDir, { recursive: true, force: true });
          resolve();
        });
      }),
  };
}

/** Une requête brute, pour maîtriser l'en-tête Host. */
export function rawGet(port: number, path: string, headers: Record<string, string>): Promise<{ status: number; body: string }> {
  return new Promise((resolve, reject) => {
    const req = request({ host: '127.0.0.1', port, path, headers }, (res) => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (chunk: string) => (body += chunk));
      res.on('end', () => resolve({ status: res.statusCode ?? 0, body }));
    });
    req.on('error', reject);
    req.end();
  });
}

export interface AuthorizeParams {
  verifier: string;
  state: string;
  query: URLSearchParams;
}

export function authorizeParams(overrides: Record<string, string | null> = {}): AuthorizeParams {
  const verifier = randomToken();
  const state = randomToken();
  const values: Record<string, string | null> = {
    response_type: 'code',
    client_id: CLIENT_ID,
    redirect_uri: REDIRECT,
    code_challenge: s256(verifier),
    code_challenge_method: 'S256',
    state,
    scope: 'read offline_access',
    resource: RESOURCE,
    ...overrides,
  };
  const query = new URLSearchParams();
  for (const [name, value] of Object.entries(values)) {
    if (value !== null) {
      query.set(name, value);
    }
  }
  return { verifier, state, query };
}

export interface ConsentForm {
  requestId: string;
  csrf: string;
  cookie: string;
}

export async function openConsent(h: Harness, params: AuthorizeParams): Promise<ConsentForm> {
  const response = await fetch(`${h.base}/authorize?${params.query}`, { redirect: 'manual' });
  if (response.status !== 200) {
    throw new Error(`consentement : ${response.status}`);
  }
  const html = await response.text();
  const field = (name: string) => new RegExp(`name="${name}" value="([^"]+)"`).exec(html)?.[1] ?? '';
  const cookie = (response.headers.get('set-cookie') ?? '').split(';')[0] ?? '';
  return { requestId: field('request'), csrf: field('csrf'), cookie };
}

export function submitConsent(
  h: Harness,
  form: ConsentForm,
  fields: Record<string, string> = {},
  headers: Record<string, string> = {},
): Promise<Response> {
  const body = new URLSearchParams({ request: form.requestId, csrf: form.csrf, secret: SECRET, decision: 'allow', ...fields });
  return fetch(`${h.base}/authorize`, {
    method: 'POST',
    redirect: 'manual',
    headers: { 'content-type': 'application/x-www-form-urlencoded', cookie: form.cookie, ...headers },
    body,
  });
}

export function token(h: Harness, fields: Record<string, string>, contentType = 'application/x-www-form-urlencoded'): Promise<Response> {
  const body = contentType === 'application/json' ? JSON.stringify(fields) : new URLSearchParams(fields).toString();
  return fetch(`${h.base}/token`, { method: 'POST', headers: { 'content-type': contentType }, body });
}

export interface Tokens {
  access_token: string;
  refresh_token: string;
  expires_in: number;
  token_type: string;
  scope: string;
}

/** Le flux complet : consentement, code, échange avec PKCE. */
export async function login(h: Harness): Promise<Tokens> {
  const params = authorizeParams();
  const form = await openConsent(h, params);
  const redirect = await submitConsent(h, form);
  if (redirect.status !== 303) {
    throw new Error(`connexion : ${redirect.status}`);
  }
  const location = new URL(redirect.headers.get('location') ?? '');
  const response = await token(h, {
    grant_type: 'authorization_code',
    code: location.searchParams.get('code') ?? '',
    redirect_uri: REDIRECT,
    client_id: CLIENT_ID,
    code_verifier: params.verifier,
    resource: RESOURCE,
  });
  if (response.status !== 200) {
    throw new Error(`jeton : ${response.status} ${await response.text()}`);
  }
  return (await response.json()) as Tokens;
}

/** Un appel JSON-RPC brut sur /mcp, comme un client de l'ère 2025 (Claude.ai, par exemple). */
export async function rpc(h: Harness, accessToken: string | undefined, method: string, params: unknown = {}): Promise<Response> {
  return fetch(`${h.base}/mcp`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      'mcp-protocol-version': '2025-06-18',
      ...(accessToken ? { authorization: `Bearer ${accessToken}` } : {}),
    },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  });
}

/** Le premier message JSON-RPC d'une réponse, JSON ou SSE. */
export async function rpcResult(response: Response): Promise<Record<string, unknown>> {
  const text = await response.text();
  if ((response.headers.get('content-type') ?? '').includes('text/event-stream')) {
    const data = text
      .split('\n')
      .filter((line) => line.startsWith('data:'))
      .map((line) => line.slice(5).trim())
      .find((line) => line.length > 0);
    return JSON.parse(data ?? '{}') as Record<string, unknown>;
  }
  return JSON.parse(text) as Record<string, unknown>;
}
