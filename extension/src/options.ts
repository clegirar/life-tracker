// La page d'options : l'adresse du NAS, le token de la source `twitch`, le bouton « Tester », et l'état
// de l'envoi. Le token n'est jamais réaffiché : un champ vide à l'enregistrement garde celui qui existe.

import { normalizeOrigin } from "./lib/client.js";
import type { LastError, Stats } from "./lib/queue.js";
import { CONFIG_KEY, type Config, type TestResult } from "./messages.js";

const TWITCH_ORIGINS = ["https://www.twitch.tv/*", "https://clips.twitch.tv/*"];

const $ = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (element === null) throw new Error(`élément manquant : ${id}`);
  return element as T;
};

const form = $<HTMLFormElement>("settings");
const originInput = $<HTMLInputElement>("origin");
const tokenInput = $<HTMLInputElement>("token");
const testButton = $<HTMLButtonElement>("test");
const message = $<HTMLParagraphElement>("message");

const dateFormat = new Intl.DateTimeFormat("fr-FR", { dateStyle: "medium", timeStyle: "short" });

function say(text: string, kind: "ok" | "error" | "" = ""): void {
  message.textContent = text;
  message.className = kind;
}

/** Le motif d'hôte de l'origine du NAS : sans port, qu'un motif ne porte pas. */
const hostPattern = (origin: string): string => `https://${new URL(origin).hostname}/*`;

function errorText(error: { code: string; status: number | null } | null): string {
  if (error === null) return "Aucune";
  switch (error.code) {
    case "unconfigured":
      return "Renseigner l'adresse du NAS et le token.";
    case "unauthorized":
      return "Token refusé (401) : l'envoi est arrêté. Recréer le token de la source twitch sur le NAS, puis l'enregistrer ici.";
    case "network":
      return "NAS injoignable : vérifier Tailscale, l'adresse et l'accès au NAS. Nouvel essai automatique.";
    case "server":
      return `Erreur du NAS (${error.status ?? "?"}) : nouvel essai automatique.`;
    case "refused":
      return `Envoi refusé par le NAS (${error.status ?? "?"}) : nouvel essai automatique.`;
    default:
      return "Erreur inconnue.";
  }
}

async function refresh(): Promise<void> {
  const stored = await browser.storage.local.get([CONFIG_KEY, "queue", "stats"]);
  const config = stored[CONFIG_KEY] as Partial<Config> | undefined;
  const stats = (stored["stats"] ?? {}) as Partial<Stats>;
  const queueSize = Array.isArray(stored["queue"]) ? stored["queue"].length : 0;

  if (document.activeElement !== originInput && config?.origin) originInput.value = config.origin;
  tokenInput.placeholder = config?.token ? "Token enregistré (laisser vide pour le garder)" : "Collé depuis le gestionnaire de mots de passe";

  const origins = [...TWITCH_ORIGINS];
  const origin = normalizeOrigin(config?.origin ?? "");
  if (origin !== null) origins.push(hostPattern(origin));
  const granted = await browser.permissions.contains({ origins });
  const access = $("access");
  access.textContent = granted ? "Accordé" : "Manquant : enregistrer les réglages pour le demander";
  access.className = granted ? "" : "error";

  $("queue").textContent = String(queueSize);
  $("last-sent").textContent = stats.lastSentAt ? dateFormat.format(stats.lastSentAt) : "Jamais";
  $("rejected").textContent = String(stats.rejectedCount ?? 0);
  const lastError: LastError | null = stats.lastError ?? null;
  const errorElement = $("last-error");
  errorElement.textContent =
    lastError === null ? "Aucune" : `${errorText(lastError)} (${dateFormat.format(lastError.at)})`;
  errorElement.className = lastError === null ? "" : "error";
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  const origin = normalizeOrigin(originInput.value);
  if (origin === null) {
    say("L'adresse doit commencer par https://.", "error");
    return;
  }
  if (!new URL(origin).hostname.endsWith(".ts.net")) {
    say("L'adresse doit être celle du NAS sur Tailscale (…ts.net).", "error");
    return;
  }
  // La demande de permission doit partir tout de suite, dans le geste de l'utilisateur, avant tout await.
  const request = browser.permissions.request({ origins: [hostPattern(origin), ...TWITCH_ORIGINS] });
  void (async () => {
    if (!(await request.catch(() => false))) {
      say("Permission refusée : sans elle, l'extension ne peut ni mesurer ni envoyer.", "error");
      return;
    }
    const stored = await browser.storage.local.get(CONFIG_KEY);
    const previous = stored[CONFIG_KEY] as Partial<Config> | undefined;
    const token = tokenInput.value.trim() || previous?.token || "";
    const config: Config = { origin, token };
    await browser.storage.local.set({ [CONFIG_KEY]: config });
    tokenInput.value = "";
    originInput.value = origin;
    await browser.runtime.sendMessage({ type: "settings-saved" });
    say(token === "" ? "Enregistré. Il manque le token." : "Enregistré.", token === "" ? "error" : "ok");
    await refresh();
  })();
});

testButton.addEventListener("click", () => {
  testButton.disabled = true;
  say("Test en cours…");
  void (async () => {
    try {
      const result = (await browser.runtime.sendMessage({ type: "test" })) as TestResult;
      if (result.ok) say("Le NAS a bien reçu le battement de l'extension.", "ok");
      else say(errorText({ code: result.error, status: result.status ?? null }), "error");
    } catch {
      say("L'extension n'a pas répondu. La recharger depuis about:addons.", "error");
    } finally {
      testButton.disabled = false;
      await refresh();
    }
  })();
});

browser.storage.onChanged.addListener((_changes, area) => {
  if (area === "local") void refresh();
});

void refresh();
