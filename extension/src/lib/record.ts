// Un segment de lecture devient un enregistrement `twitch.watch` du contrat d'ingestion. La charge
// utile est une liste blanche : ni URL, ni titre, ni chat. Le script d'arrière-plan revérifie ici ce
// que le script de contenu lui envoie : rien d'autre que ces cinq champs ne peut partir.

import { asLogin, cleanText, type Content } from "./page.js";
import { MAX_SEGMENT_MS, type Segment } from "./segmenter.js";
import { localRfc3339 } from "./time.js";

export const KIND = "twitch.watch";
export const SCHEMA_VERSION = 1;

export interface TwitchPayload {
  channel: string | null;
  channel_name: string | null;
  content: Content;
  duration_ms: number;
  category: string | null;
}

export interface IngestRecord {
  key: string;
  kind: typeof KIND;
  schema_version: typeof SCHEMA_VERSION;
  occurred_at: string;
  payload: TwitchPayload;
}

const CONTENTS: readonly string[] = ["live", "vod", "clip"];

/** Un segment reçu d'un script de contenu (donc non fiable) : l'enregistrement, ou `null` s'il est invalide. */
export function toRecord(
  segment: unknown,
  key: string,
  format: (ms: number) => string = localRfc3339,
): IngestRecord | null {
  if (typeof segment !== "object" || segment === null) return null;
  const { startMs, durationMs, context } = segment as Partial<Segment>;
  if (typeof startMs !== "number" || !Number.isFinite(startMs) || startMs <= 0) return null;
  if (typeof durationMs !== "number" || !Number.isFinite(durationMs)) return null;
  const duration = Math.round(durationMs);
  if (duration < 1 || duration > MAX_SEGMENT_MS) return null;
  if (typeof context !== "object" || context === null) return null;
  if (typeof context.content !== "string" || !CONTENTS.includes(context.content)) return null;
  const channel = typeof context.channel === "string" ? asLogin(context.channel) : null;
  if (context.channel !== null && channel === null) return null;
  const text = (value: unknown): string | null => (typeof value === "string" ? cleanText(value) : null);
  return {
    key,
    kind: KIND,
    schema_version: SCHEMA_VERSION,
    occurred_at: format(Math.round(startMs)),
    payload: {
      channel,
      channel_name: channel === null ? null : text(context.channelName),
      content: context.content,
      duration_ms: duration,
      category: text(context.category),
    },
  };
}
