// Le script d'arrière-plan (page d'événements, mise en veille par Firefox entre deux réveils) :
// - reçoit les segments des scripts de contenu, les met en forme et les range dans la file ;
// - vide la file chaque minute par alarme, et quelques secondes après chaque segment ;
// - bat toutes les 15 min (`POST /v1/heartbeat`) et rapporte son état (`PUT /v1/sources/me/status`) ;
// - affiche « ! » sur son bouton quand le token est refusé.
// Les appels partent d'ici seulement, avec la permission d'hôte de la seule origine du NAS.

import { Client, normalizeOrigin, type Failure } from "./lib/client.js";
import { Queue, statusFor, type Stats } from "./lib/queue.js";
import { toRecord } from "./lib/record.js";
import { CONFIG_KEY, type Config, type Message, type TestResult } from "./messages.js";

const FLUSH = "flush";
const FLUSH_SOON = "flush-soon";
const HEARTBEAT = "heartbeat";
const FLUSH_SOON_MINUTES = 5 / 60;

const VERSION = `twitch-extension-${browser.runtime.getManifest().version}`;
const queue = new Queue(browser.storage.local, Date.now);

async function client(): Promise<Client | null> {
  const stored = await browser.storage.local.get(CONFIG_KEY);
  const config = stored[CONFIG_KEY] as Partial<Config> | undefined;
  const origin = normalizeOrigin(config?.origin ?? "");
  const token = config?.token?.trim() ?? "";
  if (origin === null || token === "") return null;
  return new Client({ origin, token, version: VERSION }, (url, init) => fetch(url, init));
}

async function showBadge(stats: Stats): Promise<void> {
  await browser.action.setBadgeText({ text: stats.stopped ? "!" : "" });
  if (stats.stopped) await browser.action.setBadgeBackgroundColor({ color: "#d70015" });
}

async function report(api: Client): Promise<Failure | null> {
  const outcome = await api.putStatus(statusFor(await queue.snapshot()));
  return outcome.kind === "ok" ? null : outcome;
}

async function flush(): Promise<void> {
  const before = await queue.snapshot();
  const api = await client();
  const result = await queue.flush(api);
  const after = await queue.snapshot();
  // L'état ne part que s'il change (envoi réussi ↔ en échec) : le battement le redit toutes les 15 min.
  if (api !== null && result !== "unauthorized" && (before.lastError === null) !== (after.lastError === null)) {
    const failure = await report(api);
    if (failure?.kind === "unauthorized") await queue.recordFailure(failure);
  }
  await showBadge(await queue.snapshot());
}

/** Battement, file, état. Avec `force` (bouton « Tester »), même après un 401. */
async function beat(force: boolean): Promise<TestResult> {
  const api = await client();
  if (api === null) return { ok: false, error: "unconfigured" };
  if (force) await queue.resume();
  else if ((await queue.snapshot()).stopped) return { ok: false, error: "unauthorized" };

  const fail = async (failure: Failure): Promise<TestResult> => {
    await queue.recordFailure(failure);
    await showBadge(await queue.snapshot());
    return failure.kind === "unauthorized" || failure.kind === "network"
      ? { ok: false, error: failure.kind }
      : { ok: false, error: failure.kind, status: failure.status };
  };
  const heartbeat = await api.heartbeat();
  if (heartbeat.kind !== "ok") return fail(heartbeat);
  await queue.reachable();
  await queue.flush(api);
  const failure = await report(api);
  if (failure !== null) return fail(failure);
  await showBadge(await queue.snapshot());
  return { ok: true };
}

async function ensureAlarms(): Promise<void> {
  if ((await browser.alarms.get(FLUSH)) === undefined) {
    await browser.alarms.create(FLUSH, { periodInMinutes: 1 });
  }
  if ((await browser.alarms.get(HEARTBEAT)) === undefined) {
    await browser.alarms.create(HEARTBEAT, { delayInMinutes: 0.5, periodInMinutes: 15 });
  }
}

browser.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === FLUSH || alarm.name === FLUSH_SOON) void flush();
  if (alarm.name === HEARTBEAT) void beat(false);
});

const fromExtension = (sender: browser.runtime.MessageSender): boolean =>
  sender.url?.startsWith(browser.runtime.getURL("")) === true;

function fromTwitch(sender: browser.runtime.MessageSender): boolean {
  if (sender.tab === undefined || sender.url === undefined) return false;
  try {
    return ["www.twitch.tv", "clips.twitch.tv"].includes(new URL(sender.url).hostname);
  } catch {
    return false;
  }
}

browser.runtime.onMessage.addListener((raw: unknown, sender: browser.runtime.MessageSender) => {
  if (sender.id !== browser.runtime.id || typeof raw !== "object" || raw === null) return undefined;
  const message = raw as Message;
  switch (message.type) {
    case "segment": {
      // Seuls les scripts de contenu de Twitch envoient des segments.
      if (!fromTwitch(sender)) return undefined;
      const record = toRecord(message.segment, crypto.randomUUID());
      if (record === null) return undefined;
      return queue.enqueue(record).then(async () => {
        await browser.alarms.create(FLUSH_SOON, { delayInMinutes: FLUSH_SOON_MINUTES });
      });
    }
    case "settings-saved":
      if (!fromExtension(sender)) return undefined;
      return queue.resume().then(async (stats) => {
        await showBadge(stats);
        await flush();
      });
    case "test":
      if (!fromExtension(sender)) return undefined;
      return beat(true);
    default:
      return undefined;
  }
});

browser.action.onClicked.addListener(() => {
  void browser.runtime.openOptionsPage();
});

void ensureAlarms();
void queue.snapshot().then(showBadge);
