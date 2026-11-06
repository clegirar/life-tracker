// Les trois appels de l'extension à l'API d'ingestion du NAS : un lot de segments, le battement et
// l'état de la source. Chaque appel porte le token de la source `twitch` et la version de l'extension.
// `fetch` est passé en argument : les tests le remplacent. Aucun corps de réponse n'est conservé.

import type { IngestRecord } from "./record.js";

export type Fetch = (url: string, init: RequestInit) => Promise<Response>;

export interface ClientConfig {
  /** L'origine du NAS, `https://<nas>.<tailnet>.ts.net:8443`, sans chemin. */
  origin: string;
  token: string;
  version: string;
}

/** Ce qui s'est passé, sans le texte de la réponse. */
export type Failure =
  | { kind: "unauthorized" }
  | { kind: "network" }
  | { kind: "server"; status: number }
  | { kind: "refused"; status: number };

export type Outcome<T> = { kind: "ok"; value: T } | Failure;

export interface RecordResult {
  key: string | null;
  status: "accepted" | "duplicate" | "rejected";
  reason?: string;
  field?: string;
}

/** L'état rapporté pour la page d'état : le vocabulaire fermé de `PUT /v1/sources/me/status`. */
export type SourceStatus =
  | { state: "ok"; reason: null; last_collected_at: string | null }
  | { state: "error"; reason: "ingest_failed"; last_collected_at: string | null };

export const MAX_BATCH = 500;

/** L'origine `https` d'une adresse saisie, ou `null` (http refusé : le token ne voyage qu'en TLS). */
export function normalizeOrigin(input: string): string | null {
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    return null;
  }
  if (url.protocol !== "https:" || url.username !== "" || url.password !== "") return null;
  return url.origin;
}

export class Client {
  constructor(
    private readonly config: ClientConfig,
    private readonly fetchFn: Fetch,
  ) {}

  private async call(method: string, path: string, body?: unknown): Promise<Outcome<Response>> {
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.config.token}`,
      "X-Client-Version": this.config.version,
    };
    if (body !== undefined) headers["Content-Type"] = "application/json";
    let response: Response;
    try {
      response = await this.fetchFn(`${this.config.origin}${path}`, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        credentials: "omit",
        cache: "no-store",
        redirect: "error",
      });
    } catch {
      return { kind: "network" };
    }
    if (response.status === 401) return { kind: "unauthorized" };
    if (response.status >= 500) return { kind: "server", status: response.status };
    if (!response.ok) return { kind: "refused", status: response.status };
    return { kind: "ok", value: response };
  }

  /** `POST /v1/records` : un résultat par enregistrement, dans l'ordre du lot. */
  async postRecords(records: readonly IngestRecord[]): Promise<Outcome<RecordResult[]>> {
    const outcome = await this.call("POST", "/v1/records", { records });
    if (outcome.kind !== "ok") return outcome;
    try {
      const body = (await outcome.value.json()) as { results?: unknown };
      if (!Array.isArray(body.results)) return { kind: "server", status: outcome.value.status };
      return { kind: "ok", value: body.results as RecordResult[] };
    } catch {
      return { kind: "server", status: outcome.value.status };
    }
  }

  /** `POST /v1/heartbeat` : Firefox est ouvert et l'extension en marche. */
  async heartbeat(): Promise<Outcome<null>> {
    const outcome = await this.call("POST", "/v1/heartbeat");
    return outcome.kind === "ok" ? { kind: "ok", value: null } : outcome;
  }

  /** `PUT /v1/sources/me/status`. */
  async putStatus(status: SourceStatus): Promise<Outcome<null>> {
    const outcome = await this.call("PUT", "/v1/sources/me/status", status);
    return outcome.kind === "ok" ? { kind: "ok", value: null } : outcome;
  }
}
