// Le secret de connexion du MCP : Argon2id, comparaison en temps constant, limitation globale des essais.

import { randomBytes, timingSafeEqual } from 'node:crypto';
import { argon2id } from 'hash-wasm';

/** Paramètres de l'OWASP pour Argon2id : 19 Mio, 2 passes, 1 voie. Ceux du haché font foi à la vérification. */
const MEMORY_KIB = 19456;
const ITERATIONS = 2;
const PARALLELISM = 1;
const HASH_BYTES = 32;
export const MIN_SECRET_CHARS = 16;

export async function hashSecret(secret: string): Promise<string> {
  if ([...secret].length < MIN_SECRET_CHARS) {
    throw new Error(`secret trop court : ${MIN_SECRET_CHARS} caractères au moins`);
  }
  return argon2id({
    password: secret,
    salt: randomBytes(16),
    parallelism: PARALLELISM,
    iterations: ITERATIONS,
    memorySize: MEMORY_KIB,
    hashLength: HASH_BYTES,
    outputType: 'encoded',
  });
}

interface Phc {
  memory: number;
  iterations: number;
  parallelism: number;
  salt: Buffer;
  hash: Buffer;
}

function parsePhc(encoded: string): Phc | undefined {
  // $argon2id$v=19$m=19456,t=2,p=1$<sel>$<haché>, en base64 sans remplissage.
  const parts = encoded.split('$');
  if (parts.length !== 6 || parts[1] !== 'argon2id' || parts[2] !== 'v=19') {
    return undefined;
  }
  const params = Object.fromEntries((parts[3] ?? '').split(',').map((p) => p.split('=')));
  const memory = Number(params.m);
  const iterations = Number(params.t);
  const parallelism = Number(params.p);
  if (![memory, iterations, parallelism].every((n) => Number.isInteger(n) && n > 0)) {
    return undefined;
  }
  return {
    memory,
    iterations,
    parallelism,
    salt: Buffer.from(parts[4] ?? '', 'base64'),
    hash: Buffer.from(parts[5] ?? '', 'base64'),
  };
}

/** Recalcule avec le sel et les paramètres du haché, puis compare en temps constant. */
export async function verifySecret(secret: string, encoded: string): Promise<boolean> {
  const phc = parsePhc(encoded);
  if (!phc || phc.hash.length === 0) {
    return false;
  }
  const computed = await argon2id({
    password: secret,
    salt: phc.salt,
    parallelism: phc.parallelism,
    iterations: phc.iterations,
    memorySize: phc.memory,
    hashLength: phc.hash.length,
    outputType: 'binary',
  });
  return timingSafeEqual(Buffer.from(computed), phc.hash);
}

/**
 * Limitation globale, et non par adresse : sur Internet, l'adresse ne dirait rien (relais Funnel), et il
 * n'y a qu'un seul compte. Au-delà de `max` échecs sur la fenêtre, tout essai est refusé, même juste.
 */
export class FailureLimiter {
  private failures: number[] = [];

  constructor(
    private readonly max: number,
    private readonly windowMs: number,
    private readonly now: () => number,
  ) {}

  private prune(): void {
    const since = this.now() - this.windowMs;
    this.failures = this.failures.filter((at) => at > since);
  }

  blocked(): boolean {
    this.prune();
    return this.failures.length >= this.max;
  }

  fail(): void {
    this.prune();
    this.failures.push(this.now());
  }
}
