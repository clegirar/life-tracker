import assert from "node:assert/strict";
import { test } from "node:test";

import { localRfc3339, rfc3339WithOffset } from "../src/lib/time.js";

const MS = Date.UTC(2026, 10, 6, 20, 14, 3, 250); // 6 nov. 2026, 20:14:03.250 UTC

test("l'heure porte le décalage local de l'instant", () => {
  assert.equal(rfc3339WithOffset(MS, 60), "2026-11-06T21:14:03.250+01:00");
  assert.equal(rfc3339WithOffset(MS, 120), "2026-11-06T22:14:03.250+02:00");
  assert.equal(rfc3339WithOffset(MS, 0), "2026-11-06T20:14:03.250+00:00");
  assert.equal(rfc3339WithOffset(MS, -330), "2026-11-06T14:44:03.250-05:30");
});

test("le décalage peut changer de jour", () => {
  assert.equal(rfc3339WithOffset(Date.UTC(2026, 11, 31, 23, 30), 60), "2027-01-01T00:30:00.000+01:00");
});

test("le fuseau de Paris suit l'heure d'été (TZ=Europe/Paris dans test.sh)", () => {
  if (process.env["TZ"] !== "Europe/Paris") return;
  assert.equal(localRfc3339(MS), "2026-11-06T21:14:03.250+01:00");
  // 25 oct. 2026 : 00:59:59 UTC est encore en heure d'été, 01:00 UTC est en heure d'hiver.
  assert.equal(localRfc3339(Date.UTC(2026, 9, 25, 0, 59, 59)), "2026-10-25T02:59:59.000+02:00");
  assert.equal(localRfc3339(Date.UTC(2026, 9, 25, 1, 0, 0)), "2026-10-25T02:00:00.000+01:00");
});

test("le même instant, relu, donne le même instant", () => {
  for (const offset of [60, 120, 0, -480]) {
    assert.equal(Date.parse(rfc3339WithOffset(MS, offset)), MS);
  }
});
