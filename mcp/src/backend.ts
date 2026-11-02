// Le seul accès du MCP aux données : l'API de lecture du backend, avec le jeton de service (lecture
// seule, niveau « pseudonymes »). Que des GET : aucune écriture n'est possible d'ici.

export class BackendError extends Error {
  constructor(
    readonly status: number,
    readonly field: string | undefined,
  ) {
    super(`backend ${status}`);
  }
}

export type Query = Record<string, string | number | undefined>;

export class Backend {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
    private readonly timeoutMs = 30_000,
  ) {}

  async get<T>(path: string, query: Query = {}): Promise<T> {
    const url = new URL(path, this.baseUrl);
    for (const [name, value] of Object.entries(query)) {
      if (value !== undefined) {
        url.searchParams.set(name, String(value));
      }
    }
    const response = await fetch(url, {
      method: 'GET',
      headers: { authorization: `Bearer ${this.token}`, 'x-client-version': 'mcp-1' },
      signal: AbortSignal.timeout(this.timeoutMs),
      redirect: 'error',
    });
    if (!response.ok) {
      let field: string | undefined;
      try {
        const body = (await response.json()) as { field?: unknown };
        field = typeof body.field === 'string' ? body.field : undefined;
      } catch {
        field = undefined;
      }
      throw new BackendError(response.status, field);
    }
    return (await response.json()) as T;
  }
}
