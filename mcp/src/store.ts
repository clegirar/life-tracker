// Les jetons OAuth, hachés en SHA-256 dans un petit fichier JSON du volume du conteneur. Jamais dans
// PostgreSQL : seul le backend touche la base. Une seule autorisation à la fois : une nouvelle
// connexion remplace la précédente.

import { createHash, randomBytes } from 'node:crypto';
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export const ACCESS_TOKEN_MS = 60 * 60 * 1000;
export const REFRESH_TOKEN_MS = 30 * 24 * 60 * 60 * 1000;
/** Les anciens jetons de rafraîchissement retenus pour détecter un rejeu. */
const RETIRED_KEPT = 200;

export function sha256(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

/** 32 octets aléatoires en base64url : jetons et codes. */
export function randomToken(): string {
  return randomBytes(32).toString('base64url');
}

interface TokenRecord {
  hash: string;
  expiresAt: number;
}

export interface Grant {
  clientId: string;
  scope: string;
  resource: string;
  createdAt: number;
  access: TokenRecord;
  refresh: TokenRecord;
  /** Hachés des jetons de rafraîchissement déjà échangés : en présenter un révoque l'autorisation. */
  retiredRefresh: string[];
}

interface StoreFile {
  version: 1;
  grant: Grant | null;
}

export interface IssuedTokens {
  accessToken: string;
  refreshToken: string;
  expiresInSeconds: number;
  scope: string;
}

export type RefreshOutcome =
  | { kind: 'ok'; tokens: IssuedTokens }
  | { kind: 'invalid' }
  | { kind: 'replayed' };

export interface AccessInfo {
  clientId: string;
  scope: string;
  resource: string;
  expiresAt: number;
}

export class TokenStore {
  private grant: Grant | null = null;
  private readonly path: string | null;

  /** `dir` absent : en mémoire seulement (tests). */
  constructor(
    dir: string | null,
    private readonly now: () => number,
  ) {
    this.path = dir === null ? null : join(dir, 'tokens.json');
    if (dir !== null) {
      mkdirSync(dir, { recursive: true, mode: 0o700 });
      this.grant = this.load();
    }
  }

  private load(): Grant | null {
    if (this.path === null) {
      return null;
    }
    try {
      const file = JSON.parse(readFileSync(this.path, 'utf8')) as StoreFile;
      return file.version === 1 ? file.grant : null;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
        // Un fichier illisible ne coûte qu'une nouvelle connexion depuis Claude.
        console.error(JSON.stringify({ level: 'warn', msg: 'token store unreadable, starting empty' }));
      }
      return null;
    }
  }

  /** Écriture atomique : un fichier temporaire, puis un renommage. */
  private save(): void {
    if (this.path === null) {
      return;
    }
    const file: StoreFile = { version: 1, grant: this.grant };
    const temporary = `${this.path}.tmp`;
    writeFileSync(temporary, JSON.stringify(file), { mode: 0o600 });
    renameSync(temporary, this.path);
  }

  private issue(previous: Grant | null, base: Omit<Grant, 'access' | 'refresh' | 'retiredRefresh'>): IssuedTokens {
    const accessToken = randomToken();
    const refreshToken = randomToken();
    const now = this.now();
    const retired = previous ? [...previous.retiredRefresh, previous.refresh.hash].slice(-RETIRED_KEPT) : [];
    this.grant = {
      ...base,
      access: { hash: sha256(accessToken), expiresAt: now + ACCESS_TOKEN_MS },
      refresh: { hash: sha256(refreshToken), expiresAt: now + REFRESH_TOKEN_MS },
      retiredRefresh: retired,
    };
    this.save();
    return {
      accessToken,
      refreshToken,
      expiresInSeconds: ACCESS_TOKEN_MS / 1000,
      scope: base.scope,
    };
  }

  /** Une nouvelle connexion : l'autorisation précédente disparaît avec ses jetons. */
  authorize(clientId: string, scope: string, resource: string): IssuedTokens {
    return this.issue(null, { clientId, scope, resource, createdAt: this.now() });
  }

  /** Rotation à chaque usage. Un ancien jeton présenté de nouveau révoque toute l'autorisation. */
  refresh(refreshToken: string, clientId: string): RefreshOutcome {
    const grant = this.grant;
    if (grant === null) {
      return { kind: 'invalid' };
    }
    const hash = sha256(refreshToken);
    if (grant.retiredRefresh.includes(hash)) {
      this.revoke();
      return { kind: 'replayed' };
    }
    if (hash !== grant.refresh.hash || grant.clientId !== clientId || grant.refresh.expiresAt <= this.now()) {
      return { kind: 'invalid' };
    }
    const { clientId: id, scope, resource, createdAt } = grant;
    return { kind: 'ok', tokens: this.issue(grant, { clientId: id, scope, resource, createdAt }) };
  }

  /** Le jeton d'accès courant, expiré compris : c'est au contrôle du SDK de refuser un jeton expiré. */
  access(accessToken: string): AccessInfo | undefined {
    const grant = this.grant;
    if (grant === null || sha256(accessToken) !== grant.access.hash) {
      return undefined;
    }
    return {
      clientId: grant.clientId,
      scope: grant.scope,
      resource: grant.resource,
      expiresAt: grant.access.expiresAt,
    };
  }

  revoke(): void {
    this.grant = null;
    this.save();
  }

  /** Pour les tests : l'autorisation telle que stockée. */
  current(): Grant | null {
    return this.grant;
  }

  /** Pour les tests : remplace l'autorisation stockée. */
  replace(grant: Grant | null): void {
    this.grant = grant;
    this.save();
  }
}
