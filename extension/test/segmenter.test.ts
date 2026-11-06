import assert from "node:assert/strict";
import { test } from "node:test";

import type { WatchContext } from "../src/lib/page.js";
import { MAX_SEGMENT_MS, Segmenter, type Segment } from "../src/lib/segmenter.js";

const T0 = Date.UTC(2026, 10, 6, 20, 0, 0);
const A: WatchContext = { channel: "chaine_a", channelName: "Chaine_A", content: "live", category: "Art", routeKey: "live:chaine_a" };
const B: WatchContext = { channel: "chaine_b", channelName: null, content: "live", category: null, routeKey: "live:chaine_b" };

/** Un `<video>` simulé : chaque seconde d'horloge fait avancer `currentTime` d'autant (vitesse ×1). */
function setup() {
  const segments: Segment[] = [];
  const segmenter = new Segmenter((s) => segments.push(s));
  let now = T0;
  let position = 100;
  const play = (seconds: number, context: WatchContext | null = A, id = "v") => {
    for (let i = 0; i < seconds; i++) {
      now += 1000;
      position += 1;
      segmenter.observe(id, { nowMs: now, currentTime: position, playing: true, context });
    }
  };
  return {
    segments,
    segmenter,
    play,
    start(context: WatchContext | null = A, id = "v") {
      segmenter.observe(id, { nowMs: now, currentTime: position, playing: true, context });
    },
    wait(ms: number) {
      now += ms;
    },
    tick(sample: { advance?: number; playing?: boolean; context?: WatchContext | null; elapsed?: number }, id = "v") {
      now += sample.elapsed ?? 1000;
      position += sample.advance ?? 1;
      segmenter.observe(id, {
        nowMs: now,
        currentTime: position,
        playing: sample.playing ?? true,
        context: sample.context === undefined ? A : sample.context,
      });
    },
    get now() {
      return now;
    },
    get position() {
      return position;
    },
  };
}

const spans = (segments: Segment[]) => segments.map((s) => [s.startMs - T0, s.durationMs]);

test("une lecture continue : des segments de 60 s qui se suivent, puis le reste à la pause", () => {
  const v = setup();
  v.start();
  v.play(150);
  v.segmenter.close("v", v.now, v.position);
  assert.deepEqual(spans(v.segments), [
    [0, MAX_SEGMENT_MS],
    [60_000, MAX_SEGMENT_MS],
    [120_000, 30_000],
  ]);
  assert.deepEqual(v.segments[0]?.context, A);
});

test("la pause n'est pas comptée", () => {
  const v = setup();
  v.start();
  v.play(10);
  v.segmenter.close("v", v.now, v.position);
  for (let i = 0; i < 20; i++) v.tick({ advance: 0, playing: false });
  // L'événement `playing` de la reprise ouvre le segment sans attendre la seconde suivante.
  v.start();
  v.play(5);
  v.segmenter.close("v", v.now, v.position);
  assert.deepEqual(spans(v.segments), [
    [0, 10_000],
    [30_000, 5_000],
  ]);
});

test("un trou de plus de 15 s n'est jamais compté (veille, onglet gelé)", () => {
  const v = setup();
  v.start();
  v.play(10);
  v.tick({ elapsed: 40_000, advance: 40 });
  v.play(5);
  v.segmenter.close("v", v.now, v.position);
  assert.deepEqual(spans(v.segments), [
    [0, 10_000],
    [50_000, 5_000],
  ]);
  // 15 s pile restent comptées : le seuil est strict.
  const w = setup();
  w.start();
  w.play(5);
  w.tick({ elapsed: 15_000, advance: 15 });
  w.segmenter.close("v", w.now, w.position);
  assert.deepEqual(spans(w.segments), [[0, 20_000]]);
});

test("seul compte le temps où currentTime avance", () => {
  const v = setup();
  v.start();
  v.play(10);
  // Chargement sans événement : la position ne bouge plus.
  v.tick({ advance: 0 });
  v.tick({ advance: 0 });
  v.play(5);
  v.segmenter.close("v", v.now, v.position);
  assert.deepEqual(spans(v.segments), [
    [0, 10_000],
    [12_000, 5_000],
  ]);
});

test("un saut dans la vidéo coupe le segment, en avant comme en arrière", () => {
  const v = setup();
  v.start();
  v.play(10);
  v.tick({ advance: 600 });
  v.play(10);
  v.tick({ advance: -300 });
  v.play(10);
  v.segmenter.close("v", v.now, v.position);
  assert.deepEqual(spans(v.segments), [
    [0, 10_000],
    [11_000, 10_000],
    [22_000, 10_000],
  ]);
});

test("une lecture accélérée (×2) reste comptée au temps de l'horloge", () => {
  const v = setup();
  v.start();
  for (let i = 0; i < 10; i++) v.tick({ advance: 2 });
  v.segmenter.close("v", v.now, v.position);
  assert.deepEqual(spans(v.segments), [[0, 10_000]]);
});

test("le changement de chaîne ou de contenu coupe le segment", () => {
  const v = setup();
  v.start(A);
  v.play(10, A);
  v.play(10, B);
  v.play(10, { ...B, content: "vod", routeKey: "vod:1" });
  v.segmenter.close("v", v.now, v.position);
  assert.deepEqual(
    v.segments.map((s) => [s.startMs - T0, s.durationMs, s.context.channel, s.context.content]),
    [
      [0, 10_000, "chaine_a", "live"],
      [11_000, 9_000, "chaine_b", "live"],
      [21_000, 9_000, "chaine_b", "vod"],
    ],
  );
});

test("le nom et la catégorie arrivés en retard ne coupent pas le segment", () => {
  const v = setup();
  const early = { ...A, channelName: null, category: null };
  v.start(early);
  v.play(5, early);
  v.play(5, A);
  v.segmenter.close("v", v.now, v.position);
  assert.equal(v.segments.length, 1);
  assert.deepEqual(v.segments[0]?.context, A);
});

test("la fermeture compte le dernier bout si la vidéo avançait encore", () => {
  const v = setup();
  v.start();
  v.play(10);
  // Pause 400 ms après la dernière observation, la position a avancé d'autant.
  v.segmenter.close("v", v.now + 400, v.position + 0.4);
  assert.deepEqual(spans(v.segments), [[0, 10_400]]);
});

test("une vidéo disparue ferme son segment à sa dernière observation", () => {
  const v = setup();
  v.start();
  v.play(10);
  v.segmenter.remove("v", v.now + 900, null);
  assert.deepEqual(spans(v.segments), [[0, 10_000]]);
  // Plus rien n'est ouvert : une seconde fermeture ne rend rien.
  v.segmenter.close("v", v.now + 2000, null);
  assert.equal(v.segments.length, 1);
});

test("page masquée : tout est fermé, puis la mesure reprend en arrière-plan", () => {
  const v = setup();
  v.start(A, "v1");
  v.start(B, "v2");
  v.play(10, A, "v1");
  v.segmenter.observe("v2", { nowMs: v.now, currentTime: v.position, playing: true, context: B });
  v.segmenter.closeAll(v.now, () => v.position);
  v.play(5, A, "v1");
  v.segmenter.close("v1", v.now, v.position);
  assert.deepEqual(
    v.segments.map((s) => [s.context.channel, s.startMs - T0, s.durationMs]),
    [
      ["chaine_a", 0, 10_000],
      ["chaine_b", 0, 10_000],
      ["chaine_a", 11_000, 4_000],
    ],
  );
});

test("deux vidéos en même temps ont chacune leur segment, sur le même intervalle", () => {
  const segments: Segment[] = [];
  const segmenter = new Segmenter((s) => segments.push(s));
  for (let i = 0; i <= 20; i++) {
    segmenter.observe("v1", { nowMs: T0 + i * 1000, currentTime: 10 + i, playing: true, context: A });
    segmenter.observe("v2", { nowMs: T0 + i * 1000 + 3, currentTime: 500 + i, playing: true, context: B });
  }
  segmenter.closeAll(T0 + 20_003, () => null);
  assert.deepEqual(
    segments.map((s) => [s.context.channel, s.startMs - T0, s.durationMs]),
    [
      ["chaine_a", 0, 20_000],
      ["chaine_b", 3, 20_000],
    ],
  );
});

test("moins d'une seconde ne vaut pas un enregistrement ; sans contexte, rien", () => {
  const v = setup();
  v.start();
  v.segmenter.close("v", v.now + 500, v.position + 0.5);
  v.start(null);
  v.play(10, null);
  v.segmenter.close("v", v.now, v.position);
  assert.deepEqual(v.segments, []);
});
