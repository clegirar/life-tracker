// Faux stockage et faux `fetch` pour les tests de la file et du client.
import { Client, type Fetch } from "../src/lib/client.js";
import type { Storage } from "../src/lib/queue.js";
import { toRecord, type IngestRecord } from "../src/lib/record.js";
import { rfc3339WithOffset } from "../src/lib/time.js";

export class MemoryStorage implements Storage {
  readonly data = new Map<string, unknown>();
  async get(keys: string[]): Promise<Record<string, unknown>> {
    const out: Record<string, unknown> = {};
    for (const key of keys) if (this.data.has(key)) out[key] = structuredClone(this.data.get(key));
    return out;
  }
  async set(items: Record<string, unknown>): Promise<void> {
    for (const [key, value] of Object.entries(items)) this.data.set(key, structuredClone(value));
  }
}

export interface Call {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: unknown;
}

/** Un faux NAS : chaque appel est noté, et `respond` décide de la réponse. */
export function fakeFetch(respond: (call: Call) => Response | Error): { fetch: Fetch; calls: Call[] } {
  const calls: Call[] = [];
  const fetch: Fetch = async (url, init) => {
    const call: Call = {
      url,
      method: init.method ?? "GET",
      headers: { ...(init.headers as Record<string, string>) },
      body: typeof init.body === "string" ? JSON.parse(init.body) : undefined,
    };
    calls.push(call);
    const response = respond(call);
    if (response instanceof Error) throw response;
    return response;
  };
  return { fetch, calls };
}

export const json = (status: number, body: unknown): Response =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

/** Le NAS qui accepte tout, ou rend `duplicate` pour les clés déjà vues. */
export function acceptingNas(): { fetch: Fetch; calls: Call[]; seen: Set<string> } {
  const seen = new Set<string>();
  const fake = fakeFetch((call) => {
    if (call.url.endsWith("/v1/records")) {
      const { records } = call.body as { records: IngestRecord[] };
      return json(200, {
        request_id: "00000000-0000-4000-8000-000000000000",
        results: records.map((r) => {
          const status = seen.has(r.key) ? "duplicate" : "accepted";
          seen.add(r.key);
          return { key: r.key, status };
        }),
      });
    }
    if (call.url.endsWith("/v1/heartbeat")) return json(200, { request_id: "00000000-0000-4000-8000-000000000000" });
    return new Response(null, { status: 204 });
  });
  return { ...fake, seen };
}

export const ORIGIN = "https://nas.exemple.ts.net:8443";
export const TOKEN = "jeton-fictif-de-test";
export const client = (fetch: Fetch): Client => new Client({ origin: ORIGIN, token: TOKEN, version: "twitch-extension-0.1.0" }, fetch);

let counter = 0;
/** Un enregistrement construit comme le fait l'extension, avec une clé déterministe. */
export function record(overrides: { channel?: string | null; durationMs?: number; startMs?: number } = {}): IngestRecord {
  counter += 1;
  const r = toRecord(
    {
      startMs: overrides.startMs ?? Date.UTC(2026, 10, 6, 20, 14, 3, 250),
      durationMs: overrides.durationMs ?? 60_000,
      context: {
        channel: overrides.channel === undefined ? "chaine_fictive" : overrides.channel,
        channelName: "Chaîne_Fictive",
        content: "live",
        category: "Just Chatting",
        routeKey: "live:chaine_fictive",
      },
    },
    `0192f0a6-1c3e-7d2a-9b41-${String(counter).padStart(12, "0")}`,
    (ms) => rfc3339WithOffset(ms, 60),
  );
  if (r === null) throw new Error("enregistrement invalide");
  return r;
}
