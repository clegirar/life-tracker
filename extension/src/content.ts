// Le script de contenu de www.twitch.tv et clips.twitch.tv (cadre principal seulement). Il observe
// chaque `<video>` de la page chaque seconde et à chaque événement média, et remet les segments
// fermés au script d'arrière-plan. Il ne lit que la position et l'état de lecture des vidéos, l'URL de
// la page et les quelques éléments de `SELECTORS` ; il n'envoie rien d'autre qu'un segment.

import { bindContext, miniPlayerContext, readContext, type WatchContext } from "./lib/page.js";
import { Segmenter, type Segment } from "./lib/segmenter.js";
import type { SegmentMessage } from "./messages.js";

const TICK_MS = 1_000;
/** Événements qui ferment le segment : pause, chargement, fin, saut, changement de source. */
const CLOSING = ["pause", "waiting", "ended", "seeking", "emptied", "abort", "error"] as const;

interface Tracked {
  id: string;
  video: HTMLVideoElement;
  context: WatchContext | null;
}

const tracked = new Map<HTMLVideoElement, Tracked>();
let nextId = 0;

function send(segment: Segment): void {
  const message: SegmentMessage = { type: "segment", segment };
  browser.runtime.sendMessage(message).catch(() => {
    // Arrière-plan rechargé (mise à jour de l'extension) : ce segment est perdu, une minute au plus.
  });
}

const segmenter = new Segmenter(send);

const isPlaying = (video: HTMLVideoElement): boolean => !video.paused && !video.ended && !video.seeking;

function observe(entry: Tracked, pageContext: WatchContext | null): void {
  entry.context = bindContext(entry.context, pageContext, entry.context === null ? miniPlayerContext(entry.video) : null);
  segmenter.observe(entry.id, {
    nowMs: Date.now(),
    currentTime: entry.video.currentTime,
    playing: isPlaying(entry.video),
    context: entry.context,
  });
}

function track(video: HTMLVideoElement): Tracked {
  const entry: Tracked = { id: String(nextId++), video, context: null };
  tracked.set(video, entry);
  for (const type of CLOSING) {
    video.addEventListener(type, () => segmenter.close(entry.id, Date.now(), video.currentTime));
  }
  // Reprise de la lecture : on n'attend pas la seconde suivante pour ouvrir le segment.
  video.addEventListener("playing", () => observe(entry, readContext(location.href, document)));
  return entry;
}

function tick(): void {
  const pageContext = readContext(location.href, document);
  const present = new Set<HTMLVideoElement>(document.querySelectorAll("video"));
  for (const [video, entry] of tracked) {
    if (!present.has(video) || !video.isConnected) {
      // Vidéo disparue (navigation, mini-lecteur fermé) : son segment se ferme à la dernière observation.
      segmenter.remove(entry.id, Date.now(), null);
      tracked.delete(video);
    }
  }
  for (const video of present) observe(tracked.get(video) ?? track(video), pageContext);
}

function closeAll(): void {
  const now = Date.now();
  const byId = new Map([...tracked.values()].map((entry) => [entry.id, entry.video]));
  segmenter.closeAll(now, (id) => byId.get(id)?.currentTime ?? null);
}

document.addEventListener("visibilitychange", () => {
  // Masquée, la page peut être gelée ou fermée sans prévenir : on remet ce qui est mesuré. La mesure
  // continue en arrière-plan à l'observation suivante.
  if (document.visibilityState === "hidden") closeAll();
});
window.addEventListener("pagehide", closeAll);

setInterval(tick, TICK_MS);
tick();
