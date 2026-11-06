// Les messages entre les scripts de l'extension (`runtime.sendMessage`).

import type { Segment } from "./lib/segmenter.js";

/** Script de contenu → arrière-plan : un segment fermé. */
export interface SegmentMessage {
  type: "segment";
  segment: Segment;
}

/** Page d'options → arrière-plan : réglages enregistrés, ou bouton « Tester ». */
export interface SettingsMessage {
  type: "settings-saved" | "test";
}

export type Message = SegmentMessage | SettingsMessage;

/** Réponse au bouton « Tester ». */
export type TestResult =
  | { ok: true }
  | { ok: false; error: "unconfigured" | "unauthorized" | "network" | "server" | "refused"; status?: number };

/** Les réglages, dans `storage.local` sous la clé `config`. */
export interface Config {
  origin: string;
  token: string;
}

export const CONFIG_KEY = "config";
