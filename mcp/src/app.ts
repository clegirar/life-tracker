// Le serveur HTTP : la ressource MCP protégée par OAuth, et un mini serveur d'autorisation intégré,
// à enregistrement fermé (un seul client, Claude, par son document CIMD, sans le récupérer).
//
// Sur Internet (Tailscale Funnel) ne servent que /mcp (jeton exigé), /token et /.well-known/*. La page de
// connexion refuse toute requête venue par Funnel, reconnue à l'absence de Tailscale-User-Login : elle ne
// s'ouvre que depuis le tailnet.

import { createHash, timingSafeEqual } from 'node:crypto';

import { createMcpExpressApp, mcpAuthMetadataRouter, requireBearerAuth } from '@modelcontextprotocol/express';
import { toNodeHandler } from '@modelcontextprotocol/node';
import {
  type AuthInfo,
  createMcpHandler,
  getOAuthProtectedResourceMetadataUrl,
  OAuthError,
  OAuthErrorCode,
  type OAuthMetadata,
} from '@modelcontextprotocol/server';
import express, { type NextFunction, type Request, type Response } from 'express';

import { Backend } from './backend.js';
import { type Config } from './config.js';
import { consentPage, messagePage } from './pages.js';
import { FailureLimiter, verifySecret } from './secret.js';
import { randomToken, sha256, TokenStore } from './store.js';
import { buildServer } from './tools.js';

/** Le seul client accepté : Claude, identifié par l'URL de son document de métadonnées (CIMD). */
export const CLIENT_ID = 'https://claude.ai/oauth/mcp-oauth-client-metadata';
/** Ses adresses de retour, codées en dur : le document n'est jamais récupéré (aucune sortie Internet). */
export const REDIRECT_URIS = ['https://claude.ai/api/mcp/auth_callback', 'https://claude.com/api/mcp/auth_callback'];
export const SCOPES = ['read', 'offline_access'];
export const FUNNEL_HEADER = 'tailscale-funnel-request';
/** Posé par tailscaled pour un utilisateur du tailnet (et retiré s'il vient du client) ; absent par Funnel. */
export const TAILNET_USER_HEADER = 'tailscale-user-login';
export const CSRF_COOKIE = 'lt_mcp_csrf';
export const LOGIN_MAX_FAILURES = 5;
export const LOGIN_WINDOW_MS = 15 * 60 * 1000;
const CODE_MS = 60 * 1000;
const PENDING_MS = 10 * 60 * 1000;
const MAX_PENDING = 20;

export interface Deps {
  now: () => number;
  store: TokenStore;
  backend: Backend;
  log: (entry: Record<string, unknown>) => void;
}

interface Pending {
  redirectUri: string;
  state: string | undefined;
  codeChallenge: string;
  scope: string;
  resource: string;
  csrf: string;
  expiresAt: number;
}

interface IssuedCode {
  redirectUri: string;
  codeChallenge: string;
  scope: string;
  resource: string;
  expiresAt: number;
  used: boolean;
}

/** Une valeur de paramètre : une chaîne seule, jamais un tableau ni un objet. */
function param(source: unknown, name: string): string | undefined {
  const value = (source as Record<string, unknown> | undefined)?.[name];
  return typeof value === 'string' ? value : undefined;
}

function sameText(a: string, b: string): boolean {
  const left = Buffer.from(a);
  const right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
}

function cookie(req: Request, name: string): string | undefined {
  for (const part of (req.headers.cookie ?? '').split(';')) {
    const [key, ...rest] = part.trim().split('=');
    if (key === name) {
      return rest.join('=');
    }
  }
  return undefined;
}

function withoutTrailingSlash(url: string): string {
  return url.replace(/\/$/, '');
}

export function s256(verifier: string): string {
  return createHash('sha256').update(verifier).digest('base64url');
}

export function createApp(config: Config, deps: Deps): express.Express {
  const { now, store, backend, log } = deps;
  const limiter = new FailureLimiter(LOGIN_MAX_FAILURES, LOGIN_WINDOW_MS, now);
  const pending = new Map<string, Pending>();
  const codes = new Map<string, IssuedCode>();
  // Argon2 consomme 19 Mio par calcul : une vérification à la fois.
  let verifying: Promise<unknown> = Promise.resolve();
  const verify = (secret: string): Promise<boolean> => {
    const result = verifying.then(() => verifySecret(secret, config.loginHash));
    verifying = result.catch(() => undefined);
    return result;
  };

  const metadataUrl = getOAuthProtectedResourceMetadataUrl(new URL(config.resource));
  const oauthMetadata = {
    issuer: config.publicOrigin,
    authorization_endpoint: `${config.publicOrigin}/authorize`,
    token_endpoint: `${config.publicOrigin}/token`,
    response_types_supported: ['code'],
    response_modes_supported: ['query'],
    grant_types_supported: ['authorization_code', 'refresh_token'],
    code_challenge_methods_supported: ['S256'],
    token_endpoint_auth_methods_supported: ['none'],
    scopes_supported: SCOPES,
    client_id_metadata_document_supported: true,
    authorization_response_iss_parameter_supported: true,
  } as OAuthMetadata;

  const app = createMcpExpressApp({
    host: config.listenHost,
    allowedHosts: [config.publicHostname],
    allowedOrigins: [config.publicHostname],
    jsonLimit: '1mb',
  });
  app.disable('x-powered-by');
  app.set('trust proxy', false);

  // Le seul journal d'accès : méthode, chemin sans requête, statut, durée. Jamais d'en-tête (jetons,
  // adresse transmise par Tailscale), jamais de paramètre (codes, état), jamais de corps.
  app.use((req: Request, res: Response, next: NextFunction) => {
    const started = now();
    res.on('finish', () =>
      log({
        msg: 'request completed',
        method: req.method,
        path: req.originalUrl.split('?')[0]?.slice(0, 80),
        status: res.statusCode,
        duration_ms: now() - started,
      }),
    );
    res.set({
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
      // same-origin et non no-referrer : sinon le navigateur poste le formulaire avec « Origin: null »,
      // que la protection contre le DNS rebinding refuse. Rien ne part vers une autre origine.
      'referrer-policy': 'same-origin',
      'x-frame-options': 'DENY',
    });
    next();
  });

  app.use(
    mcpAuthMetadataRouter({
      oauthMetadata,
      resourceServerUrl: new URL(config.resource),
      scopesSupported: ['read'],
      resourceName: 'Life tracker',
    }),
  );

  // ─── Connexion, depuis le tailnet seulement ────────────────────────────────

  // Le paquet Tailscale du NAS ne pose pas l'en-tête de Funnel : on exige donc aussi l'identité qu'il
  // ajoute pour un utilisateur du tailnet. Sans elle, la requête vient d'Internet.
  const fromFunnel = (req: Request, res: Response): boolean => {
    if (req.headers[FUNNEL_HEADER] === undefined && (req.headers[TAILNET_USER_HEADER] ?? '') !== '') {
      return false;
    }
    res
      .status(403)
      .type('html')
      .send(messagePage('Connexion depuis le tailnet', 'Cette page ne s’ouvre que depuis un appareil relié au tailnet, Tailscale actif.'));
    return true;
  };

  const html = (res: Response, status: number, body: string) =>
    res
      .status(status)
      .set(
        'content-security-policy',
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self' https://claude.ai https://claude.com; frame-ancestors 'none'; base-uri 'none'",
      )
      .type('html')
      .send(body);

  const redirectWith = (res: Response, redirectUri: string, values: Record<string, string | undefined>) => {
    const target = new URL(redirectUri);
    for (const [name, value] of Object.entries(values)) {
      if (value !== undefined) {
        target.searchParams.set(name, value);
      }
    }
    target.searchParams.set('iss', config.publicOrigin);
    res.redirect(303, target.href);
  };

  const prunePending = () => {
    for (const [id, entry] of pending) {
      if (entry.expiresAt <= now()) {
        pending.delete(id);
      }
    }
    while (pending.size >= MAX_PENDING) {
      const oldest = pending.keys().next().value;
      if (oldest === undefined) {
        break;
      }
      pending.delete(oldest);
    }
  };

  const csrfCookie = (value: string, maxAgeSeconds: number) =>
    `${CSRF_COOKIE}=${value}; Path=/authorize; Max-Age=${maxAgeSeconds}; HttpOnly; Secure; SameSite=Strict`;

  app.get('/authorize', (req: Request, res: Response) => {
    if (fromFunnel(req, res)) {
      return;
    }
    const q = req.query;
    const clientId = param(q, 'client_id');
    const redirectUri = param(q, 'redirect_uri');
    // Client ou retour inconnus : jamais de redirection, la page s'arrête là (RFC 6749, 4.1.2.1).
    if (clientId !== CLIENT_ID) {
      html(res, 400, messagePage('Client inconnu', 'Seul Claude peut se connecter à ce serveur.'));
      return;
    }
    if (redirectUri === undefined || !REDIRECT_URIS.includes(redirectUri)) {
      html(res, 400, messagePage('Retour refusé', 'Cette adresse de retour n’est pas celle de Claude.'));
      return;
    }
    const state = param(q, 'state');
    if (param(q, 'response_type') !== 'code') {
      redirectWith(res, redirectUri, { error: 'unsupported_response_type', state });
      return;
    }
    const challenge = param(q, 'code_challenge');
    if (param(q, 'code_challenge_method') !== 'S256' || challenge === undefined || !/^[A-Za-z0-9_-]{43}$/.test(challenge)) {
      redirectWith(res, redirectUri, { error: 'invalid_request', error_description: 'PKCE S256 required', state });
      return;
    }
    const resource = param(q, 'resource');
    if (resource !== undefined && withoutTrailingSlash(resource) !== withoutTrailingSlash(config.resource)) {
      redirectWith(res, redirectUri, { error: 'invalid_target', state });
      return;
    }
    // Une portée inconnue est ignorée ; la lecture est toujours accordée, et rien d'autre n'existe.
    const requested = (param(q, 'scope') ?? '').split(' ').filter(Boolean);
    const scope = ['read', ...SCOPES.filter((s) => s !== 'read' && requested.includes(s))].join(' ');

    prunePending();
    const requestId = randomToken();
    const csrf = randomToken();
    pending.set(requestId, {
      redirectUri,
      state,
      codeChallenge: challenge,
      scope,
      resource: config.resource,
      csrf,
      expiresAt: now() + PENDING_MS,
    });
    res.set('set-cookie', csrfCookie(csrf, PENDING_MS / 1000));
    html(res, 200, consentPage({ requestId, csrf, redirectHost: new URL(redirectUri).host, scope }));
  });

  app.post('/authorize', express.urlencoded({ extended: false, limit: '4kb' }), async (req: Request, res: Response) => {
    if (fromFunnel(req, res)) {
      return;
    }
    const requestId = param(req.body, 'request') ?? '';
    const entry = pending.get(requestId);
    if (entry === undefined || entry.expiresAt <= now()) {
      pending.delete(requestId);
      html(res, 400, messagePage('Demande expirée', 'Relance la connexion depuis Claude.'));
      return;
    }
    const csrf = param(req.body, 'csrf') ?? '';
    const fromCookie = cookie(req, CSRF_COOKIE) ?? '';
    if (!sameText(csrf, entry.csrf) || !sameText(fromCookie, entry.csrf)) {
      html(res, 403, messagePage('Demande refusée', 'Relance la connexion depuis Claude.'));
      return;
    }
    const page = (status: number, error: string) =>
      html(
        res,
        status,
        consentPage({ requestId, csrf: entry.csrf, redirectHost: new URL(entry.redirectUri).host, scope: entry.scope, error }),
      );
    if (param(req.body, 'decision') === 'deny') {
      pending.delete(requestId);
      res.set('set-cookie', csrfCookie('', 0));
      redirectWith(res, entry.redirectUri, { error: 'access_denied', state: entry.state });
      return;
    }
    if (limiter.blocked()) {
      log({ msg: 'login throttled' });
      page(429, 'Trop d’essais : réessaie dans un quart d’heure.');
      return;
    }
    const secret = param(req.body, 'secret') ?? '';
    if (secret.length > 1024 || !(await verify(secret))) {
      limiter.fail();
      log({ msg: 'login refused' });
      page(401, 'Secret incorrect.');
      return;
    }
    pending.delete(requestId);
    const code = randomToken();
    codes.set(sha256(code), {
      redirectUri: entry.redirectUri,
      codeChallenge: entry.codeChallenge,
      scope: entry.scope,
      resource: entry.resource,
      expiresAt: now() + CODE_MS,
      used: false,
    });
    log({ msg: 'login succeeded' });
    res.set('set-cookie', csrfCookie('', 0));
    redirectWith(res, entry.redirectUri, { code, state: entry.state });
  });

  // ─── Jetons ────────────────────────────────────────────────────────────────

  const tokenError = (res: Response, status: number, error: string, description?: string) =>
    res
      .status(status)
      .set('pragma', 'no-cache')
      .json(description ? { error, error_description: description } : { error });

  app.post('/token', express.urlencoded({ extended: false, limit: '8kb' }), (req: Request, res: Response) => {
    if (!req.is('application/x-www-form-urlencoded')) {
      tokenError(res, 400, 'invalid_request', 'form-urlencoded body required');
      return;
    }
    for (const [id, issued] of codes) {
      if (issued.expiresAt <= now() && !issued.used) {
        codes.delete(id);
      }
    }
    const body = req.body as unknown;
    if (param(body, 'client_id') !== CLIENT_ID) {
      tokenError(res, 401, 'invalid_client');
      return;
    }
    const grantType = param(body, 'grant_type');
    const resource = param(body, 'resource');
    if (resource !== undefined && withoutTrailingSlash(resource) !== withoutTrailingSlash(config.resource)) {
      tokenError(res, 400, 'invalid_target');
      return;
    }
    if (grantType === 'authorization_code') {
      const id = sha256(param(body, 'code') ?? '');
      const issued = codes.get(id);
      if (issued === undefined || issued.expiresAt <= now()) {
        tokenError(res, 400, 'invalid_grant');
        return;
      }
      if (issued.used) {
        // Un code rejoué : l'autorisation qu'il a donnée tombe avec lui (RFC 6749, 4.1.2).
        codes.delete(id);
        store.revoke();
        log({ msg: 'authorization code replayed, grant revoked' });
        tokenError(res, 400, 'invalid_grant');
        return;
      }
      issued.used = true;
      const verifier = param(body, 'code_verifier') ?? '';
      if (
        param(body, 'redirect_uri') !== issued.redirectUri ||
        !/^[A-Za-z0-9._~-]{43,128}$/.test(verifier) ||
        !sameText(s256(verifier), issued.codeChallenge)
      ) {
        tokenError(res, 400, 'invalid_grant');
        return;
      }
      const tokens = store.authorize(CLIENT_ID, issued.scope, issued.resource);
      log({ msg: 'tokens issued' });
      res.set('pragma', 'no-cache').json({
        access_token: tokens.accessToken,
        token_type: 'Bearer',
        expires_in: tokens.expiresInSeconds,
        refresh_token: tokens.refreshToken,
        scope: tokens.scope,
      });
      return;
    }
    if (grantType === 'refresh_token') {
      const outcome = store.refresh(param(body, 'refresh_token') ?? '', CLIENT_ID);
      if (outcome.kind !== 'ok') {
        if (outcome.kind === 'replayed') {
          log({ msg: 'refresh token replayed, grant revoked' });
        }
        tokenError(res, 400, 'invalid_grant');
        return;
      }
      log({ msg: 'tokens refreshed' });
      res.set('pragma', 'no-cache').json({
        access_token: outcome.tokens.accessToken,
        token_type: 'Bearer',
        expires_in: outcome.tokens.expiresInSeconds,
        refresh_token: outcome.tokens.refreshToken,
        scope: outcome.tokens.scope,
      });
      return;
    }
    tokenError(res, 400, 'unsupported_grant_type');
  });

  // ─── La ressource MCP ──────────────────────────────────────────────────────

  const bearer = requireBearerAuth({
    verifier: {
      verifyAccessToken: async (token: string): Promise<AuthInfo> => {
        const info = store.access(token);
        if (info === undefined) {
          throw new OAuthError(OAuthErrorCode.InvalidToken, 'Invalid token');
        }
        return {
          token,
          clientId: info.clientId,
          scopes: info.scope.split(' '),
          expiresAt: Math.floor(info.expiresAt / 1000),
          resource: new URL(info.resource),
        };
      },
    },
    resourceMetadataUrl: metadataUrl,
    expectedResource: new URL(config.resource),
  });
  // Sans état : une instance par requête, pour l'ère 2026 comme pour les clients 2025 (repli par défaut).
  const handler = createMcpHandler(() => buildServer(backend), {
    onerror: (error) => log({ level: 'warn', msg: 'mcp error', error: error.name }),
    maxRequestBodySize: 1_000_000,
  });
  const serveMcp = toNodeHandler(handler);
  app.all('/mcp', bearer, (req: Request, res: Response) => {
    void serveMcp(req, res, req.method === 'POST' ? req.body : undefined);
  });

  // Rien d'autre ne répond.
  app.use((_req: Request, res: Response) => {
    res.status(404).json({ error: 'not_found' });
  });
  app.use((error: unknown, _req: Request, res: Response, _next: NextFunction) => {
    const status = (error as { status?: number }).status;
    if (typeof status === 'number' && status >= 400 && status < 500) {
      res.status(status).json({ error: 'invalid_request' });
      return;
    }
    log({ level: 'error', msg: 'unhandled error', error: error instanceof Error ? error.name : 'unknown' });
    res.status(500).json({ error: 'server_error' });
  });

  return app;
}
