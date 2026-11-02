// Le serveur ne répond à rien sans OAuth : découverte, refus sans jeton, jeton expiré ou d'une autre
// audience, client et retour inconnus, PKCE, rejeu, Funnel, CSRF, limitation des essais.

import assert from 'node:assert/strict';
import { readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { after, before, describe, test } from 'node:test';

import { CLIENT_ID, LOGIN_WINDOW_MS, REDIRECT_URIS } from '../src/app.js';
import { decodeLoginHash } from '../src/config.js';
import { hashSecret, verifySecret } from '../src/secret.js';
import {
  authorizeParams,
  TAILNET,
  type Harness,
  login,
  openConsent,
  PUBLIC_ORIGIN,
  rawGet,
  REDIRECT,
  RESOURCE,
  rpc,
  SECRET,
  startServer,
  submitConsent,
  token,
} from './helpers.js';

const METADATA_URL = `${PUBLIC_ORIGIN}/.well-known/oauth-protected-resource/mcp`;

let h: Harness;
before(async () => {
  h = await startServer();
});
after(async () => {
  await h.close();
});

describe('découverte', () => {
  test('les métadonnées de la ressource nomment exactement l’URL du connecteur', async () => {
    const response = await fetch(`${h.base}/.well-known/oauth-protected-resource/mcp`);
    assert.equal(response.status, 200);
    const body = (await response.json()) as Record<string, unknown>;
    assert.equal(body.resource, RESOURCE);
    assert.deepEqual(body.authorization_servers, [PUBLIC_ORIGIN]);
  });

  test('le serveur d’autorisation : CIMD, PKCE S256, client public, sans enregistrement', async () => {
    const response = await fetch(`${h.base}/.well-known/oauth-authorization-server`);
    assert.equal(response.status, 200);
    const body = (await response.json()) as Record<string, unknown>;
    assert.equal(body.issuer, PUBLIC_ORIGIN);
    assert.equal(body.authorization_endpoint, `${PUBLIC_ORIGIN}/authorize`);
    assert.equal(body.token_endpoint, `${PUBLIC_ORIGIN}/token`);
    assert.equal(body.client_id_metadata_document_supported, true);
    assert.deepEqual(body.code_challenge_methods_supported, ['S256']);
    assert.deepEqual(body.token_endpoint_auth_methods_supported, ['none']);
    assert.ok((body.scopes_supported as string[]).includes('offline_access'));
    assert.equal(body.registration_endpoint, undefined);
  });

  test('la découverte reste servie par Funnel', async () => {
    const response = await fetch(`${h.base}/.well-known/oauth-authorization-server`, {
      headers: { 'tailscale-funnel-request': '?1' },
    });
    assert.equal(response.status, 200);
  });
});

describe('rien sans jeton', () => {
  test('/mcp sans jeton : 401 et l’adresse des métadonnées', async () => {
    for (const method of ['POST', 'GET', 'DELETE']) {
      const response = await fetch(`${h.base}/mcp`, { method });
      assert.equal(response.status, 401, method);
      const challenge = response.headers.get('www-authenticate') ?? '';
      assert.match(challenge, /^Bearer /);
      assert.ok(challenge.includes(`resource_metadata="${METADATA_URL}"`), challenge);
    }
  });

  test('un faux jeton ou un autre schéma : 401', async () => {
    for (const authorization of ['Bearer inconnu', 'Basic dXNlcjpwYXNz', 'Bearer']) {
      const response = await fetch(`${h.base}/mcp`, { method: 'POST', headers: { authorization } });
      assert.equal(response.status, 401, authorization);
    }
    const response = await rpc(h, 'inconnu', 'tools/list');
    assert.equal(response.status, 401);
  });

  test('toute autre route : 404, sans rien dire', async () => {
    for (const path of ['/', '/register', '/v1/days/2026-10-16', '/health', '/.well-known/openid-configuration']) {
      const response = await fetch(`${h.base}${path}`);
      assert.equal(response.status, 404, path);
    }
  });

  test('un autre nom d’hôte est refusé (DNS rebinding)', async () => {
    const response = await rawGet(h.port, '/.well-known/oauth-authorization-server', { host: 'evil.example' });
    assert.equal(response.status, 403);
  });
});

describe('connexion', () => {
  test('la page de connexion refuse Funnel, en GET comme en POST', async () => {
    const params = authorizeParams();
    const viaFunnel = await fetch(`${h.base}/authorize?${params.query}`, {
      headers: { ...TAILNET, 'tailscale-funnel-request': '?1' },
      redirect: 'manual',
    });
    assert.equal(viaFunnel.status, 403);
    // Le paquet Tailscale du NAS ne pose pas l'en-tête de Funnel : sans identité du tailnet, refusé aussi.
    const anonymous = await fetch(`${h.base}/authorize?${params.query}`, { redirect: 'manual' });
    assert.equal(anonymous.status, 403);
    const form = await openConsent(h, params);
    const post = await submitConsent(h, form, {}, { 'tailscale-funnel-request': '?1' });
    assert.equal(post.status, 403);
    assert.equal(post.headers.get('location'), null);
    const anonymousPost = await submitConsent(h, form, {}, { 'tailscale-user-login': '' });
    assert.equal(anonymousPost.status, 403);
  });

  test('le formulaire posté par un navigateur, avec sa propre origine, passe', async () => {
    const response = await fetch(`${h.base}/authorize?${authorizeParams().query}`, { headers: TAILNET });
    assert.equal(response.headers.get('referrer-policy'), 'same-origin');
    // Une balise no-referrer ferait poster le formulaire avec « Origin: null » (vu dans Chromium sur le NAS).
    assert.ok(!(await response.text()).includes('no-referrer'));
    const form = await openConsent(h, authorizeParams());
    const post = await submitConsent(h, form, {}, { origin: PUBLIC_ORIGIN });
    assert.equal(post.status, 303);
  });

  test('un client inconnu ou un retour hors liste : refus sans redirection', async () => {
    const cases: Record<string, string | null>[] = [
      { client_id: 'https://evil.example/client.json' },
      { client_id: null },
      { redirect_uri: 'https://evil.example/callback' },
      { redirect_uri: 'https://claude.ai/api/mcp/auth_callback/../evil' },
      { redirect_uri: null },
    ];
    for (const overrides of cases) {
      const response = await fetch(`${h.base}/authorize?${authorizeParams(overrides).query}`, { redirect: 'manual', headers: TAILNET });
      assert.equal(response.status, 400, JSON.stringify(overrides));
      assert.equal(response.headers.get('location'), null);
    }
  });

  test('sans PKCE S256 ou pour une autre ressource : erreur renvoyée à Claude', async () => {
    for (const [overrides, error] of [
      [{ code_challenge: null }, 'invalid_request'],
      [{ code_challenge_method: 'plain' }, 'invalid_request'],
      [{ code_challenge_method: null }, 'invalid_request'],
      [{ response_type: 'token' }, 'unsupported_response_type'],
      [{ resource: 'https://evil.example/mcp' }, 'invalid_target'],
    ] as const) {
      const params = authorizeParams(overrides);
      const response = await fetch(`${h.base}/authorize?${params.query}`, { redirect: 'manual', headers: TAILNET });
      assert.equal(response.status, 303, JSON.stringify(overrides));
      const location = new URL(response.headers.get('location') ?? '');
      assert.equal(`${location.origin}${location.pathname}`, REDIRECT);
      assert.equal(location.searchParams.get('error'), error);
      assert.equal(location.searchParams.get('state'), params.state);
      assert.equal(location.searchParams.get('code'), null);
    }
  });

  test('la page montre l’hôte de retour et pose un cookie CSRF strict', async () => {
    const response = await fetch(`${h.base}/authorize?${authorizeParams({ redirect_uri: REDIRECT_URIS[1]! }).query}`, { headers: TAILNET });
    assert.equal(response.status, 200);
    const html = await response.text();
    assert.ok(html.includes('claude.com'));
    const cookie = response.headers.get('set-cookie') ?? '';
    for (const attribute of ['HttpOnly', 'Secure', 'SameSite=Strict', 'Path=/authorize']) {
      assert.ok(cookie.includes(attribute), cookie);
    }
    assert.match(response.headers.get('content-security-policy') ?? '', /frame-ancestors 'none'/);
  });

  test('CSRF : sans le cookie ou avec un autre jeton, refusé', async () => {
    const form = await openConsent(h, authorizeParams());
    const noCookie = await submitConsent(h, { ...form, cookie: '' });
    assert.equal(noCookie.status, 403);
    const otherField = await submitConsent(h, { ...form, csrf: 'autre' });
    assert.equal(otherField.status, 403);
    const otherOrigin = await submitConsent(h, form, {}, { origin: 'https://evil.example' });
    assert.equal(otherOrigin.status, 403);
    // La demande reste valable pour le vrai formulaire.
    const ok = await submitConsent(h, form);
    assert.equal(ok.status, 303);
  });

  test('refuser renvoie access_denied', async () => {
    const params = authorizeParams();
    const form = await openConsent(h, params);
    const response = await submitConsent(h, form, { decision: 'deny', secret: '' });
    assert.equal(response.status, 303);
    const location = new URL(response.headers.get('location') ?? '');
    assert.equal(location.searchParams.get('error'), 'access_denied');
    assert.equal(location.searchParams.get('state'), params.state);
  });

  test('cinq échecs en quinze minutes bloquent tout, même le bon secret', async () => {
    const limited = await startServer();
    try {
      const form = await openConsent(limited, authorizeParams());
      for (let i = 0; i < 5; i += 1) {
        const wrong = await submitConsent(limited, form, { secret: `mauvais secret ${i}` });
        assert.equal(wrong.status, 401);
      }
      const blocked = await submitConsent(limited, form);
      assert.equal(blocked.status, 429);
      assert.equal(blocked.headers.get('location'), null);
      limited.clock.now += LOGIN_WINDOW_MS + 1000;
      const fresh = await openConsent(limited, authorizeParams());
      const allowed = await submitConsent(limited, fresh);
      assert.equal(allowed.status, 303);
      assert.ok(limited.logs.some((entry) => entry.msg === 'login throttled'));
    } finally {
      await limited.close();
    }
  });
});

describe('jetons', () => {
  async function code(): Promise<{ code: string; verifier: string }> {
    const params = authorizeParams();
    const form = await openConsent(h, params);
    const response = await submitConsent(h, form);
    const location = new URL(response.headers.get('location') ?? '');
    assert.equal(location.searchParams.get('iss'), PUBLIC_ORIGIN);
    return { code: location.searchParams.get('code') ?? '', verifier: params.verifier };
  }

  const exchange = (c: { code: string; verifier: string }, overrides: Record<string, string> = {}) =>
    token(h, {
      grant_type: 'authorization_code',
      code: c.code,
      redirect_uri: REDIRECT,
      client_id: CLIENT_ID,
      code_verifier: c.verifier,
      ...overrides,
    });

  test('un flux complet donne un accès d’une heure et un rafraîchissement', async () => {
    const tokens = await login(h);
    assert.equal(tokens.token_type, 'Bearer');
    assert.equal(tokens.expires_in, 3600);
    assert.ok(tokens.refresh_token.length >= 43);
    const response = await rpc(h, tokens.access_token, 'tools/list');
    assert.equal(response.status, 200);
  });

  test('un faux vérificateur PKCE est refusé, et le code est perdu', async () => {
    const c = await code();
    const wrong = await exchange(c, { code_verifier: 'x'.repeat(43) });
    assert.equal(wrong.status, 400);
    assert.deepEqual(await wrong.json(), { error: 'invalid_grant' });
    const retry = await exchange(c);
    assert.equal(retry.status, 400);
  });

  test('un autre retour, un autre client, une autre ressource : refusés', async () => {
    const c = await code();
    const otherRedirect = await exchange(c, { redirect_uri: REDIRECT_URIS[1]! });
    assert.equal(otherRedirect.status, 400);
    const other = await code();
    const otherClient = await exchange(other, { client_id: 'https://evil.example/client.json' });
    assert.equal(otherClient.status, 401);
    assert.deepEqual(await otherClient.json(), { error: 'invalid_client' });
    const otherResource = await exchange(other, { resource: 'https://evil.example/mcp' });
    assert.equal(otherResource.status, 400);
    assert.deepEqual(await otherResource.json(), { error: 'invalid_target' });
  });

  test('un code rejoué révoque l’autorisation qu’il a donnée', async () => {
    const c = await code();
    const first = await exchange(c);
    assert.equal(first.status, 200);
    const tokens = (await first.json()) as { access_token: string };
    const replay = await exchange(c);
    assert.equal(replay.status, 400);
    const response = await rpc(h, tokens.access_token, 'tools/list');
    assert.equal(response.status, 401);
  });

  test('un code expire au bout d’une minute', async () => {
    const c = await code();
    h.clock.now += 61_000;
    try {
      const response = await exchange(c);
      assert.equal(response.status, 400);
    } finally {
      h.clock.now -= 61_000;
    }
  });

  test('/token exige un formulaire, un type d’octroi connu et le client de Claude', async () => {
    const json = await token(h, { grant_type: 'refresh_token', client_id: CLIENT_ID, refresh_token: 'x' }, 'application/json');
    assert.equal(json.status, 400);
    assert.equal(((await json.json()) as { error: string }).error, 'invalid_request');
    const password = await token(h, { grant_type: 'password', client_id: CLIENT_ID });
    assert.equal(password.status, 400);
    assert.deepEqual(await password.json(), { error: 'unsupported_grant_type' });
    const noClient = await token(h, { grant_type: 'refresh_token', refresh_token: 'x' });
    assert.equal(noClient.status, 401);
    assert.equal(noClient.headers.get('cache-control'), 'no-store');
  });

  test('rafraîchir fait tourner les jetons ; rejouer un ancien révoque tout', async () => {
    const tokens = await login(h);
    const refresh = (refreshToken: string) =>
      token(h, { grant_type: 'refresh_token', refresh_token: refreshToken, client_id: CLIENT_ID, resource: RESOURCE });
    const first = await refresh(tokens.refresh_token);
    assert.equal(first.status, 200);
    const rotated = (await first.json()) as { access_token: string; refresh_token: string };
    assert.notEqual(rotated.refresh_token, tokens.refresh_token);
    assert.equal((await rpc(h, tokens.access_token, 'tools/list')).status, 401, 'l’ancien accès tombe');
    assert.equal((await rpc(h, rotated.access_token, 'tools/list')).status, 200);

    const replay = await refresh(tokens.refresh_token);
    assert.equal(replay.status, 400);
    assert.deepEqual(await replay.json(), { error: 'invalid_grant' });
    assert.equal((await rpc(h, rotated.access_token, 'tools/list')).status, 401, 'tout est révoqué');
    assert.equal((await refresh(rotated.refresh_token)).status, 400);
    assert.equal(h.store.current(), null);
  });

  test('un rafraîchissement expire au bout de trente jours', async () => {
    const tokens = await login(h);
    h.clock.now += 31 * 24 * 3600 * 1000;
    try {
      const response = await token(h, { grant_type: 'refresh_token', refresh_token: tokens.refresh_token, client_id: CLIENT_ID });
      assert.equal(response.status, 400);
      assert.deepEqual(await response.json(), { error: 'invalid_grant' });
    } finally {
      h.clock.now -= 31 * 24 * 3600 * 1000;
    }
  });

  test('une nouvelle connexion remplace la précédente', async () => {
    const first = await login(h);
    const second = await login(h);
    assert.equal((await rpc(h, first.access_token, 'tools/list')).status, 401);
    assert.equal((await rpc(h, second.access_token, 'tools/list')).status, 200);
  });

  test('un jeton expiré est refusé', async () => {
    const tokens = await login(h);
    const grant = h.store.current()!;
    h.store.replace({ ...grant, access: { ...grant.access, expiresAt: Date.now() - 1000 } });
    const response = await rpc(h, tokens.access_token, 'tools/list');
    assert.equal(response.status, 401);
    assert.match(response.headers.get('www-authenticate') ?? '', /expired/);
  });

  test('un jeton d’une autre audience est refusé', async () => {
    const tokens = await login(h);
    const grant = h.store.current()!;
    h.store.replace({ ...grant, resource: 'https://autre.example/mcp' });
    const response = await rpc(h, tokens.access_token, 'tools/list');
    assert.equal(response.status, 401);
    assert.match(response.headers.get('www-authenticate') ?? '', /not issued for this resource/);
  });

  test('jetons hachés sur le volume, fichier en 600, aucun jeton ni code dans les journaux', async () => {
    const tokens = await login(h);
    const path = join(h.stateDir, 'tokens.json');
    const stored = readFileSync(path, 'utf8');
    assert.ok(!stored.includes(tokens.access_token));
    assert.ok(!stored.includes(tokens.refresh_token));
    assert.equal(statSync(path).mode & 0o777, 0o600);
    const logs = JSON.stringify(h.logs);
    for (const secret of [tokens.access_token, tokens.refresh_token, SECRET, 'code=']) {
      assert.ok(!logs.includes(secret), secret);
    }
    assert.ok(!/127\.0\.0\.1|::1/.test(logs), 'aucune adresse');
  });
});

describe('secret de connexion', () => {
  test('Argon2id, vérification exacte, encodage sans « $ »', async () => {
    const hash = await hashSecret(SECRET);
    assert.match(hash, /^\$argon2id\$v=19\$m=19456,t=2,p=1\$/);
    assert.equal(await verifySecret(SECRET, hash), true);
    assert.equal(await verifySecret(`${SECRET} `, hash), false);
    assert.equal(await verifySecret(SECRET, 'pas un haché'), false);
    const encoded = Buffer.from(hash).toString('base64url');
    assert.ok(!encoded.includes('$'));
    assert.equal(decodeLoginHash(encoded), hash);
    await assert.rejects(hashSecret('trop court'));
  });
});
