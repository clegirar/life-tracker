import assert from "node:assert/strict";
import { test } from "node:test";

import { MAX_BATCH, normalizeOrigin } from "../src/lib/client.js";
import { FIRST_BACKOFF_MS, MAX_BACKOFF_MS, Queue, statusFor } from "../src/lib/queue.js";
import { acceptingNas, client, fakeFetch, json, MemoryStorage, ORIGIN, record, TOKEN } from "./fakes.js";

function setup() {
  const storage = new MemoryStorage();
  let now = Date.UTC(2026, 10, 6, 21, 0, 0);
  const queue = new Queue(storage, () => now);
  return {
    storage,
    queue,
    advance(ms: number) {
      now += ms;
    },
    get now() {
      return now;
    },
  };
}

test("la file est envoyée avec le token et la version, puis vidée", async () => {
  const { queue } = setup();
  const nas = acceptingNas();
  await queue.enqueue(record());
  await queue.enqueue(record());
  assert.equal((await queue.snapshot()).queueSize, 2);
  assert.equal(await queue.flush(client(nas.fetch)), "sent");
  assert.equal(nas.calls.length, 1);
  const call = nas.calls[0]!;
  assert.equal(call.url, `${ORIGIN}/v1/records`);
  assert.equal(call.method, "POST");
  assert.equal(call.headers["Authorization"], `Bearer ${TOKEN}`);
  assert.equal(call.headers["X-Client-Version"], "twitch-extension-0.1.0");
  assert.equal((call.body as { records: unknown[] }).records.length, 2);
  const snapshot = await queue.snapshot();
  assert.equal(snapshot.queueSize, 0);
  assert.notEqual(snapshot.lastSentAt, null);
  assert.equal(await queue.flush(client(nas.fetch)), "empty");
  assert.equal(nas.calls.length, 1);
});

test("par lots de 500 au plus", async () => {
  const { queue } = setup();
  const nas = acceptingNas();
  for (let i = 0; i < MAX_BATCH + 20; i++) await queue.enqueue(record());
  assert.equal(await queue.flush(client(nas.fetch)), "sent");
  assert.deepEqual(
    nas.calls.map((c) => (c.body as { records: unknown[] }).records.length),
    [MAX_BATCH, 20],
  );
});

test("un renvoi après une réponse perdue donne duplicate et sort de la file", async () => {
  const { queue } = setup();
  const nas = acceptingNas();
  const r = record();
  nas.seen.add(r.key);
  await queue.enqueue(r);
  assert.equal(await queue.flush(client(nas.fetch)), "sent");
  assert.equal((await queue.snapshot()).queueSize, 0);
});

test("un rejeté est mis de côté et compté ; une réponse sans rapport laisse dans la file", async () => {
  const { queue, storage } = setup();
  const [a, b, c] = [record(), record(), record()];
  const nas = fakeFetch(() =>
    json(200, {
      request_id: "00000000-0000-4000-8000-000000000000",
      results: [
        { key: a.key, status: "accepted" },
        { key: b.key, status: "rejected", reason: "kind_not_declared", field: "kind" },
        { key: "autre", status: "accepted" },
      ],
    }),
  );
  for (const r of [a, b, c]) await queue.enqueue(r);
  assert.equal(await queue.flush(client(nas.fetch)), "sent");
  const snapshot = await queue.snapshot();
  assert.equal(snapshot.queueSize, 1);
  assert.equal(snapshot.rejectedCount, 1);
  const rejected = storage.data.get("rejected") as Array<{ record: { key: string }; reason: string; field: string }>;
  assert.deepEqual(
    rejected.map((r) => [r.record.key, r.reason, r.field]),
    [[b.key, "kind_not_declared", "kind"]],
  );
  assert.deepEqual((storage.data.get("queue") as Array<{ key: string }>).map((r) => r.key), [c.key]);
});

test("échec réseau ou 5xx : rien ne sort, délai qui double jusqu'à 1 h", async () => {
  const t = setup();
  let fail: Response | Error = new TypeError("NetworkError");
  const nas = fakeFetch(() => fail);
  await t.queue.enqueue(record());
  assert.equal(await t.queue.flush(client(nas.fetch)), "failed");
  let snapshot = await t.queue.snapshot();
  assert.equal(snapshot.queueSize, 1);
  assert.equal(snapshot.lastError?.code, "network");
  assert.equal(snapshot.retryAt, t.now + FIRST_BACKOFF_MS);
  // Pendant le délai, rien ne part.
  assert.equal(await t.queue.flush(client(nas.fetch)), "waiting");
  assert.equal(nas.calls.length, 1);

  fail = json(503, { error: "internal" });
  const delays: number[] = [];
  for (let i = 0; i < 8; i++) {
    t.advance(MAX_BACKOFF_MS);
    assert.equal(await t.queue.flush(client(nas.fetch)), "failed");
    snapshot = await t.queue.snapshot();
    delays.push(snapshot.retryAt! - t.now);
  }
  assert.deepEqual(delays, [120_000, 240_000, 480_000, 960_000, 1_920_000, 3_600_000, 3_600_000, 3_600_000]);
  assert.deepEqual(snapshot.lastError, { code: "server", status: 503, at: t.now });
  assert.deepEqual(statusFor(snapshot), { state: "error", reason: "ingest_failed", last_collected_at: null });

  // Le NAS revient : tout part, l'erreur et le délai sont oubliés.
  t.advance(MAX_BACKOFF_MS);
  assert.equal(await t.queue.flush(client(acceptingNas().fetch)), "sent");
  snapshot = await t.queue.snapshot();
  assert.equal(snapshot.queueSize, 0);
  assert.equal(snapshot.lastError, null);
  assert.equal(snapshot.backoffMs, null);
  assert.deepEqual(statusFor(snapshot), {
    state: "ok",
    reason: null,
    last_collected_at: new Date(t.now).toISOString(),
  });
});

test("un battement réussi lève l'attente", async () => {
  const t = setup();
  await t.queue.enqueue(record());
  await t.queue.flush(client(fakeFetch(() => new TypeError("NetworkError")).fetch));
  await t.queue.reachable();
  const snapshot = await t.queue.snapshot();
  assert.equal(snapshot.retryAt, null);
  assert.equal(snapshot.lastError, null);
  assert.equal(await t.queue.flush(client(acceptingNas().fetch)), "sent");
});

test("401 : l'envoi s'arrête jusqu'aux prochains réglages", async () => {
  const t = setup();
  const refused = fakeFetch(() => json(401, { error: "unauthorized" }));
  await t.queue.enqueue(record());
  assert.equal(await t.queue.flush(client(refused.fetch)), "unauthorized");
  let snapshot = await t.queue.snapshot();
  assert.equal(snapshot.stopped, true);
  assert.equal(snapshot.lastError?.code, "unauthorized");
  t.advance(10 * MAX_BACKOFF_MS);
  assert.equal(await t.queue.flush(client(refused.fetch)), "stopped");
  assert.equal(refused.calls.length, 1);
  // Un battement réussi ne relance pas : seul un nouvel enregistrement des réglages le fait.
  await t.queue.reachable();
  assert.equal((await t.queue.snapshot()).stopped, true);
  await t.queue.resume();
  assert.equal(await t.queue.flush(client(acceptingNas().fetch)), "sent");
  snapshot = await t.queue.snapshot();
  assert.equal(snapshot.stopped, false);
  assert.equal(snapshot.queueSize, 0);
});

test("sans réglages, la file attend", async () => {
  const t = setup();
  await t.queue.enqueue(record());
  assert.equal(await t.queue.flush(null), "waiting");
  assert.equal((await t.queue.snapshot()).queueSize, 1);
});

test("un segment arrivé pendant l'envoi n'est pas perdu", async () => {
  const t = setup();
  const late = record();
  const nas = acceptingNas();
  let once = true;
  const slow = async (url: string, init: RequestInit) => {
    if (once) await t.queue.enqueue(late);
    once = false;
    return nas.fetch(url, init);
  };
  await t.queue.enqueue(record());
  // Deux vidages simultanés n'en font qu'un.
  const [first, second] = await Promise.all([t.queue.flush(client(slow)), t.queue.flush(client(slow))]);
  assert.equal(first, "sent");
  assert.equal(second, "sent");
  // Le premier lot part sans effacer le segment tardif, qui part dans un second lot du même vidage.
  assert.equal(nas.calls.length, 2);
  assert.deepEqual(
    (nas.calls[1]!.body as { records: Array<{ key: string }> }).records.map((r) => r.key),
    [late.key],
  );
  assert.equal((await t.queue.snapshot()).queueSize, 0);
});

test("battement et état : les bons appels", async () => {
  const nas = acceptingNas();
  const api = client(nas.fetch);
  assert.equal((await api.heartbeat()).kind, "ok");
  assert.equal((await api.putStatus({ state: "ok", reason: null, last_collected_at: null })).kind, "ok");
  assert.deepEqual(
    nas.calls.map((c) => [c.method, c.url.slice(ORIGIN.length), c.headers["Authorization"], c.body]),
    [
      ["POST", "/v1/heartbeat", `Bearer ${TOKEN}`, undefined],
      ["PUT", "/v1/sources/me/status", `Bearer ${TOKEN}`, { state: "ok", reason: null, last_collected_at: null }],
    ],
  );
  const down = client(fakeFetch(() => json(500, { error: "internal" })).fetch);
  assert.deepEqual(await down.heartbeat(), { kind: "server", status: 500 });
});

test("l'adresse du NAS : une origine https, sans chemin ni identifiants", () => {
  assert.equal(normalizeOrigin(" https://nas.exemple.ts.net:8443/ "), "https://nas.exemple.ts.net:8443");
  assert.equal(normalizeOrigin("https://nas.exemple.ts.net:8443/v1/records"), "https://nas.exemple.ts.net:8443");
  assert.equal(normalizeOrigin("http://nas.exemple.ts.net:8443"), null);
  assert.equal(normalizeOrigin("https://moi:secret@nas.exemple.ts.net"), null);
  assert.equal(normalizeOrigin("nas"), null);
});
