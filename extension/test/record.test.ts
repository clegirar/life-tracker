import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { test } from "node:test";

import { Ajv2020 } from "ajv/dist/2020.js";

import { toRecord } from "../src/lib/record.js";
import { rfc3339WithOffset } from "../src/lib/time.js";

// ajv-formats est en CommonJS : sa fonction est l'export lui-même.
const addFormats = createRequire(import.meta.url)("ajv-formats") as (ajv: Ajv2020) => Ajv2020;

const contract = (path: string): unknown =>
  JSON.parse(readFileSync(new URL(`../../../contract/${path}`, import.meta.url), "utf8"));

const ajv = addFormats(new Ajv2020({ allErrors: true, strict: false }));
const validate = ajv.compile(contract("ingestion-v1.schema.json") as object);

const START = Date.UTC(2026, 10, 6, 20, 14, 3, 250);
const winter = (ms: number) => rfc3339WithOffset(ms, 60);
const context = {
  channel: "chaine_fictive",
  channelName: "Chaîne_Fictive",
  content: "live",
  category: "Just Chatting",
  routeKey: "live:chaine_fictive",
};

test("un lot construit par l'extension suit le contrat d'ingestion et la fixture twitch.json", () => {
  const records = [
    toRecord({ startMs: START, durationMs: 60_000, context }, "0192f0a6-1c3e-7d2a-9b41-2f6c8e1d4a01", winter),
    toRecord(
      {
        startMs: START + 60_000,
        durationMs: 42_000,
        context: { channel: null, channelName: null, content: "vod", category: null, routeKey: "vod:1" },
      },
      "0192f0a6-1c3e-7d2a-9b41-2f6c8e1d4a02",
      winter,
    ),
  ];
  const batch = { records };
  assert.equal(validate(batch), true, JSON.stringify(validate.errors));

  // La fixture du contrat (écrite avec le backend) : même forme, champ pour champ, mêmes valeurs ici.
  const fixture = contract("fixtures/valid/twitch.json") as { records: Array<Record<string, unknown>> };
  assert.equal(validate(fixture), true);
  assert.equal(records.length, fixture.records.length);
  records.forEach((record, i) => {
    const expected = fixture.records[i]!;
    assert.deepEqual(Object.keys(record!).sort(), Object.keys(expected).sort());
    assert.deepEqual(
      Object.keys(record!.payload).sort(),
      Object.keys(expected["payload"] as object).sort(),
    );
  });
  assert.deepEqual(JSON.parse(JSON.stringify(batch)), fixture);
});

test("la charge utile est une liste blanche : rien d'autre ne part", () => {
  const record = toRecord(
    { startMs: START, durationMs: 1234.4, context: { ...context, routeKey: "vod:1", title: "Titre", url: "https://x" } },
    "k",
    winter,
  );
  assert.deepEqual(record, {
    key: "k",
    kind: "twitch.watch",
    schema_version: 1,
    occurred_at: "2026-11-06T21:14:03.250+01:00",
    payload: {
      channel: "chaine_fictive",
      channel_name: "Chaîne_Fictive",
      content: "live",
      duration_ms: 1234,
      category: "Just Chatting",
    },
  });
});

test("un segment invalide venu d'un script de contenu est refusé", () => {
  const ok = { startMs: START, durationMs: 1000, context };
  for (const bad of [
    null,
    "segment",
    { ...ok, startMs: Number.NaN },
    { ...ok, durationMs: 0 },
    { ...ok, durationMs: 60_001 },
    { ...ok, durationMs: "60000" },
    { ...ok, context: null },
    { ...ok, context: { ...context, content: "replay" } },
    { ...ok, context: { ...context, channel: "Pas un login !" } },
    { ...ok, context: { ...context, channel: 42 } },
  ]) {
    assert.equal(toRecord(bad, "k", winter), null, JSON.stringify(bad));
  }
  // Sans chaîne, pas de nom ; un nom trop long ou vide devient null.
  const unknown = toRecord({ ...ok, context: { ...context, channel: null } }, "k", winter);
  assert.equal(unknown?.payload.channel_name, null);
  const long = toRecord({ ...ok, context: { ...context, category: "x".repeat(101), channelName: "  " } }, "k", winter);
  assert.equal(long?.payload.category, null);
  assert.equal(long?.payload.channel_name, null);
});
