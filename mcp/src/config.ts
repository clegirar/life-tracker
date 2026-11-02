// Configuration du serveur MCP, lue dans l'environnement. Les secrets viennent de secrets/mcp.env
// sur le NAS (voir docs/deploiement.md), jamais du dépôt.

export interface Config {
  /** Origine publique, sans chemin : https://<machine>.<tailnet>.ts.net:8443. C'est l'émetteur OAuth. */
  publicOrigin: string;
  /** L'URL saisie dans Claude, chemin compris : la ressource protégée (RFC 9728, RFC 8707). */
  resource: string;
  /** Le nom d'hôte public, seul accepté dans l'en-tête Host (protection contre le DNS rebinding). */
  publicHostname: string;
  backendUrl: string;
  /** Jeton de service de l'API de lecture : lecture seule, niveau « pseudonymes ». */
  serviceToken: string;
  /** Haché Argon2id (format PHC) du secret de connexion propre au MCP. */
  loginHash: string;
  listenHost: string;
  listenPort: number;
  stateDir: string;
}

export const MCP_PATH = '/mcp';

function required(env: NodeJS.ProcessEnv, name: string): string {
  const value = env[name]?.trim();
  if (!value) {
    throw new Error(`${name} manquant`);
  }
  return value;
}

/** Le haché arrive encodé en base64url : un `$` dans un fichier d'environnement serait interpolé. */
export function decodeLoginHash(encoded: string): string {
  const hash = Buffer.from(encoded, 'base64url').toString('utf8');
  if (!hash.startsWith('$argon2id$')) {
    throw new Error('MCP_LOGIN_HASH illisible : le régénérer avec hash-password');
  }
  return hash;
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const origin = new URL(required(env, 'MCP_PUBLIC_URL'));
  if (origin.protocol !== 'https:') {
    throw new Error('MCP_PUBLIC_URL doit être en https');
  }
  if (origin.pathname !== '/' || origin.search || origin.hash) {
    throw new Error('MCP_PUBLIC_URL est une origine, sans chemin');
  }
  const [listenHost, listenPort] = (env.MCP_LISTEN ?? '0.0.0.0:8080').split(':');
  const backend = new URL(required(env, 'MCP_BACKEND_URL'));
  return {
    publicOrigin: origin.origin,
    resource: `${origin.origin}${MCP_PATH}`,
    publicHostname: origin.hostname,
    backendUrl: backend.origin,
    serviceToken: required(env, 'MCP_SERVICE_TOKEN'),
    loginHash: decodeLoginHash(required(env, 'MCP_LOGIN_HASH')),
    listenHost: listenHost ?? '0.0.0.0',
    listenPort: Number(listenPort ?? 8080),
    stateDir: env.MCP_STATE_DIR ?? '/var/lib/life-tracker-mcp',
  };
}
