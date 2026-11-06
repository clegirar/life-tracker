// La machine à états qui découpe la lecture d'un `<video>` en segments. Un segment est un intervalle
// continu de l'horloge, `[début, début + durée]`, pendant lequel la vidéo jouait et son `currentTime`
// avançait. Il se ferme :
// - à la pause, au chargement, à la fin, au saut dans la vidéo (événements remontés par `close`) ;
// - au changement de chaîne ou de contenu, à la disparition de la vidéo, à la page masquée ou fermée ;
// - quand `currentTime` n'avance plus, ou saute, entre deux observations ;
// - au plus tard au bout de `MAX_SEGMENT_MS` : un segment perdu (onglet tué) coûte une minute au plus.
// Un trou de plus de `MAX_GAP_MS` entre deux observations (veille, onglet gelé) n'est jamais compté.
// Pas de découpe à minuit : c'est la lecture qui coupe. Module pur : l'heure est passée en argument.

import { sameWatch, type WatchContext } from "./page.js";

export const MAX_SEGMENT_MS = 60_000;
export const MAX_GAP_MS = 15_000;
/** Un segment plus court ne vaut pas un enregistrement (un saut, un clic sur pause aussitôt repris). */
export const MIN_SEGMENT_MS = 1_000;
/** Ce que `currentTime` peut prendre d'avance sur l'horloge (vitesse ×2, rattrapage du direct). */
const MAX_DRIFT_FACTOR = 2.5;
const MAX_DRIFT_SLACK_S = 2;

export interface Segment {
  startMs: number;
  durationMs: number;
  context: WatchContext;
}

/** Une observation d'un `<video>` : l'horloge, sa position, et s'il est censé jouer. */
export interface Sample {
  nowMs: number;
  currentTime: number;
  playing: boolean;
  context: WatchContext | null;
}

interface Open {
  startMs: number;
  lastMs: number;
  lastTime: number;
  context: WatchContext;
}

/** Une piste par `<video>`, sous un identifiant choisi par l'appelant. */
export class Segmenter {
  private readonly tracks = new Map<string, Open | null>();

  constructor(private readonly emit: (segment: Segment) => void) {}

  /** Une observation : chaque seconde, et à chaque événement média qui ne ferme pas. */
  observe(id: string, sample: Sample): void {
    const open = this.tracks.get(id) ?? null;

    if (!sample.playing || sample.context === null) {
      this.finish(id, open);
      return;
    }
    if (open !== null && !sameWatch(open.context, sample.context)) {
      this.finish(id, open);
      this.start(id, sample);
      return;
    }
    if (open === null) {
      // Le premier instant qui compte est celui d'une observation où la vidéo avançait déjà : on ouvre
      // ici, et l'intervalle ne grandit qu'aux observations suivantes.
      this.start(id, sample);
      return;
    }
    const elapsedMs = sample.nowMs - open.lastMs;
    const advanced = sample.currentTime - open.lastTime;
    if (elapsedMs <= 0) return;
    if (elapsedMs > MAX_GAP_MS || !this.advancedNormally(advanced, elapsedMs)) {
      // Trou, vidéo figée ou saut : rien de l'écart n'est compté ; on repart de cette observation.
      this.finish(id, open);
      this.start(id, sample);
      return;
    }
    open.lastMs = sample.nowMs;
    open.lastTime = sample.currentTime;
    // Même chaîne, même contenu : le nom et la catégorie, s'ils arrivent en retard, sont repris.
    open.context = { ...sample.context };
    // 60 s au plus : les segments pleins se suivent sans trou, le reste continue.
    while (open.lastMs - open.startMs >= MAX_SEGMENT_MS) {
      this.emit({ startMs: open.startMs, durationMs: MAX_SEGMENT_MS, context: { ...open.context } });
      open.startMs += MAX_SEGMENT_MS;
    }
  }

  /**
   * Ferme le segment de ce `<video>` : pause, chargement, fin, saut, disparition. Si la vidéo avançait
   * encore depuis la dernière observation (`currentTime` connu), ce dernier bout compte aussi.
   */
  close(id: string, nowMs: number, currentTime: number | null): void {
    const open = this.tracks.get(id) ?? null;
    if (open !== null && currentTime !== null) {
      const elapsedMs = nowMs - open.lastMs;
      if (
        elapsedMs > 0 &&
        elapsedMs <= MAX_GAP_MS &&
        this.advancedNormally(currentTime - open.lastTime, elapsedMs)
      ) {
        open.lastMs = Math.min(nowMs, open.startMs + MAX_SEGMENT_MS);
        open.lastTime = currentTime;
      }
    }
    this.finish(id, open);
  }

  /** Oublie un `<video>` disparu, après avoir fermé son segment. */
  remove(id: string, nowMs: number, currentTime: number | null): void {
    this.close(id, nowMs, currentTime);
    this.tracks.delete(id);
  }

  /** Page masquée ou fermée : tout segment ouvert est fermé et remis aussitôt. */
  closeAll(nowMs: number, currentTimeOf: (id: string) => number | null): void {
    for (const id of this.tracks.keys()) this.close(id, nowMs, currentTimeOf(id));
  }

  private advancedNormally(advancedS: number, elapsedMs: number): boolean {
    return advancedS > 0 && advancedS <= (elapsedMs / 1000) * MAX_DRIFT_FACTOR + MAX_DRIFT_SLACK_S;
  }

  private start(id: string, sample: Sample): void {
    if (sample.context === null) return;
    this.tracks.set(id, {
      startMs: sample.nowMs,
      lastMs: sample.nowMs,
      lastTime: sample.currentTime,
      context: { ...sample.context },
    });
  }

  private finish(id: string, open: Open | null): void {
    if (open === null) return;
    this.tracks.set(id, null);
    const durationMs = Math.round(open.lastMs - open.startMs);
    if (durationMs >= MIN_SEGMENT_MS) {
      this.emit({ startMs: open.startMs, durationMs, context: open.context });
    }
  }
}
