// La file des segments à envoyer, dans `storage.local` : elle survit à la mise en veille du script
// d'arrière-plan, à la fermeture de Firefox et aux coupures du NAS. Vidée par lots de 500 au plus :
// - `accepted` et `duplicate` sortent de la file (renvoyer un segment déjà reçu ne double rien) ;
// - `rejected` est mis de côté, compté et montré par la page d'options ;
// - échec réseau, 5xx ou autre refus : rien ne sort, nouvel essai après un délai qui double, 1 h au plus ;
// - 401 : le token est refusé, l'envoi s'arrête jusqu'à ce qu'un token soit de nouveau enregistré.
// Module pur : le stockage, `fetch` (par `Client`) et l'heure sont passés en argument.

import { MAX_BATCH, type Client, type Failure, type SourceStatus } from "./client.js";
import type { IngestRecord } from "./record.js";

/** Le sous-ensemble de `browser.storage.local` utilisé. */
export interface Storage {
  get(keys: string[]): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
}

export type ErrorCode = "network" | "server" | "refused" | "unauthorized";

export interface LastError {
  code: ErrorCode;
  status: number | null;
  at: number;
}

export interface Stats {
  /** Dernier lot reçu par le NAS (au moins un segment). */
  lastSentAt: number | null;
  lastError: LastError | null;
  rejectedCount: number;
  /** Pas d'envoi avant cet instant (délai croissant après un échec). */
  retryAt: number | null;
  backoffMs: number | null;
  /** Token refusé (401) : plus rien n'est envoyé avant un nouvel enregistrement des réglages. */
  stopped: boolean;
}

export interface RejectedRecord {
  record: IngestRecord;
  reason: string | null;
  field: string | null;
  at: number;
}

export interface Snapshot extends Stats {
  queueSize: number;
}

export type FlushResult = "empty" | "sent" | "waiting" | "stopped" | "failed" | "unauthorized";

export const FIRST_BACKOFF_MS = 60_000;
export const MAX_BACKOFF_MS = 3_600_000;
/** Les rejetés gardés pour examen ; au-delà, seul le compte augmente. */
export const MAX_REJECTED_KEPT = 200;

const QUEUE = "queue";
const REJECTED = "rejected";
const STATS = "stats";

const INITIAL_STATS: Stats = {
  lastSentAt: null,
  lastError: null,
  rejectedCount: 0,
  retryAt: null,
  backoffMs: null,
  stopped: false,
};

export class Queue {
  /** Les lectures-écritures du stockage passent l'une après l'autre ; jamais pendant un appel réseau. */
  private lock: Promise<unknown> = Promise.resolve();
  private flushing: Promise<FlushResult> | null = null;

  constructor(
    private readonly storage: Storage,
    private readonly now: () => number,
  ) {}

  private exclusive<T>(task: () => Promise<T>): Promise<T> {
    const run = this.lock.then(task, task);
    this.lock = run.catch(() => undefined);
    return run;
  }

  private async read(): Promise<{ queue: IngestRecord[]; rejected: RejectedRecord[]; stats: Stats }> {
    const stored = await this.storage.get([QUEUE, REJECTED, STATS]);
    return {
      queue: Array.isArray(stored[QUEUE]) ? (stored[QUEUE] as IngestRecord[]) : [],
      rejected: Array.isArray(stored[REJECTED]) ? (stored[REJECTED] as RejectedRecord[]) : [],
      stats: { ...INITIAL_STATS, ...(stored[STATS] as Partial<Stats> | undefined) },
    };
  }

  enqueue(record: IngestRecord): Promise<void> {
    return this.exclusive(async () => {
      const { queue } = await this.read();
      queue.push(record);
      await this.storage.set({ [QUEUE]: queue });
    });
  }

  snapshot(): Promise<Snapshot> {
    return this.exclusive(async () => {
      const { queue, stats } = await this.read();
      return { ...stats, queueSize: queue.length };
    });
  }

  private updateStats(change: (stats: Stats) => Partial<Stats>): Promise<Stats> {
    return this.exclusive(async () => {
      const { stats } = await this.read();
      const next = { ...stats, ...change(stats) };
      await this.storage.set({ [STATS]: next });
      return next;
    });
  }

  /** De nouveaux réglages : l'envoi reprend sans attendre, même après un 401. */
  resume(): Promise<Stats> {
    return this.updateStats(() => ({ stopped: false, retryAt: null, backoffMs: null, lastError: null }));
  }

  /** Un échec d'appel (envoi, battement, état) : délai croissant, ou arrêt sur un 401. */
  recordFailure(failure: Failure): Promise<Stats> {
    const at = this.now();
    if (failure.kind === "unauthorized") {
      return this.updateStats(() => ({
        stopped: true,
        retryAt: null,
        lastError: { code: "unauthorized", status: 401, at },
      }));
    }
    return this.updateStats((stats) => {
      const backoffMs = Math.min(MAX_BACKOFF_MS, stats.backoffMs === null ? FIRST_BACKOFF_MS : stats.backoffMs * 2);
      return {
        backoffMs,
        retryAt: at + backoffMs,
        lastError: { code: failure.kind, status: "status" in failure ? failure.status : null, at },
      };
    });
  }

  /**
   * Le NAS vient de répondre (battement réussi) : plus d'attente, et l'erreur passée est oubliée. Le
   * prochain envoi de la file dira si elle persiste.
   */
  reachable(): Promise<Stats> {
    return this.updateStats((stats) => (stats.stopped ? {} : { retryAt: null, backoffMs: null, lastError: null }));
  }

  /** Envoie la file par lots. Un seul envoi à la fois ; un second appel attend le premier. */
  flush(client: Client | null): Promise<FlushResult> {
    if (this.flushing !== null) return this.flushing;
    this.flushing = this.flushOnce(client).finally(() => {
      this.flushing = null;
    });
    return this.flushing;
  }

  private async flushOnce(client: Client | null): Promise<FlushResult> {
    let sent = false;
    for (;;) {
      const { queue, stats } = await this.exclusive(() => this.read());
      if (stats.stopped) return "stopped";
      if (queue.length === 0) return sent ? "sent" : "empty";
      if (client === null) return "waiting";
      if (stats.retryAt !== null && this.now() < stats.retryAt) return "waiting";

      const batch = queue.slice(0, MAX_BATCH);
      const outcome = await client.postRecords(batch);
      if (outcome.kind !== "ok") {
        await this.recordFailure(outcome);
        return outcome.kind === "unauthorized" ? "unauthorized" : "failed";
      }
      const done = new Set<string>();
      const setAside: RejectedRecord[] = [];
      const at = this.now();
      batch.forEach((record, i) => {
        const result = outcome.value[i];
        // Le résultat doit être celui de cet enregistrement ; sinon il reste dans la file.
        if (result === undefined || result.key !== record.key) return;
        if (result.status === "accepted" || result.status === "duplicate") {
          done.add(record.key);
        } else if (result.status === "rejected") {
          done.add(record.key);
          setAside.push({ record, reason: result.reason ?? null, field: result.field ?? null, at });
        }
      });
      await this.exclusive(async () => {
        const current = await this.read();
        const rejected = [...current.rejected, ...setAside].slice(-MAX_REJECTED_KEPT);
        await this.storage.set({
          [QUEUE]: current.queue.filter((r) => !done.has(r.key)),
          [REJECTED]: rejected,
          [STATS]: {
            ...current.stats,
            lastSentAt: at,
            lastError: null,
            retryAt: null,
            backoffMs: null,
            rejectedCount: current.stats.rejectedCount + setAside.length,
          },
        });
      });
      sent = true;
      // Une réponse qui ne correspond à rien : on s'arrête plutôt que de renvoyer le même lot en boucle.
      if (done.size === 0) return "sent";
    }
  }
}

/** L'état à rapporter : `ok` avec le dernier envoi réussi, ou `error/ingest_failed`. */
export function statusFor(stats: Stats): SourceStatus {
  const lastCollectedAt = stats.lastSentAt === null ? null : new Date(stats.lastSentAt).toISOString();
  return stats.lastError === null
    ? { state: "ok", reason: null, last_collected_at: lastCollectedAt }
    : { state: "error", reason: "ingest_failed", last_collected_at: lastCollectedAt };
}
