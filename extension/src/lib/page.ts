// De l'URL et du DOM de Twitch à ce qui est regardé : la chaîne (login), son nom affiché, le contenu
// (live, rediffusion, clip) et la catégorie. Rien d'autre n'est lu : ni titre, ni chat, ni URL envoyée.
// Le DOM de Twitch change sans prévenir : tous les sélecteurs sont dans la seule table `SELECTORS`, et
// l'URL reste la source principale. Ce module ne touche ni à `browser` ni à `document` : il reçoit une
// racine de recherche, ce qui le rend testable sous Node.

export type Content = "live" | "vod" | "clip";

/** Ce qui est regardé dans un `<video>`. `routeKey` ne sert qu'à comparer, il n'est jamais envoyé. */
export interface WatchContext {
  channel: string | null;
  channelName: string | null;
  content: Content;
  category: string | null;
  routeKey: string;
}

/** Ce que l'URL dit seule. `none` : une page où rien ne se regarde (accueil, annuaire, réglages…). */
export interface Route {
  content: Content | "none";
  login: string | null;
  routeKey: string | null;
}

/** Le strict nécessaire de `Element`, pour que les tests n'aient pas besoin d'un vrai DOM. */
export interface ElementLike {
  getAttribute(name: string): string | null;
  readonly textContent: string | null;
}

export interface QueryRoot {
  querySelectorAll(selectors: string): Iterable<ElementLike>;
}

/** Un `<video>` vu par ce module : son conteneur de mini-lecteur, s'il y en a un. */
export interface VideoLike {
  closest(selectors: string): QueryRoot | null;
}

/**
 * Tous les sélecteurs du DOM de Twitch, essayés dans l'ordre. Hypothèses à vérifier par l'essai sur
 * le vrai Twitch (étape 0 du chantier) ; ajuster ici seulement.
 */
export const SELECTORS = {
  /** Liens vers la chaîne (`/<login>`), dans l'en-tête d'infos d'un live, d'une rediffusion ou d'un clip. */
  channelLink: [
    ".channel-info-content a[href]",
    "[data-a-target='player-info-channel-link']",
    "[data-test-selector='clip-channel-link']",
    ".clips-player-info a[href]",
  ],
  /**
   * Le nom affiché de la chaîne. Jamais `[data-a-target='stream-title']`, qui est le titre du stream.
   * Retenu seulement s'il est le login à la casse près (voir `displayName`).
   */
  channelName: [".channel-info-content h1.tw-title", ".channel-info-content h1", "h1.tw-title"],
  /** La catégorie : un lien vers l'annuaire (`/directory/category/…`), rien d'autre n'est retenu. */
  category: ["a[data-a-target='stream-game-link']", ".channel-info-content a[href^='/directory/']"],
  /** Le conteneur du lecteur qui suit la navigation (mini-lecteur), et ses liens vers la chaîne. */
  miniPlayer: [".persistent-player"],
  miniPlayerChannelLink: ["a[href]"],
} as const;

/** Premiers segments d'URL de www.twitch.tv qui ne sont pas des chaînes. */
const RESERVED = new Set([
  "",
  "activate",
  "annual-recap",
  "bits",
  "broadcast",
  "collections",
  "creatorcamp",
  "dashboard",
  "directory",
  "downloads",
  "drops",
  "embed",
  "event",
  "events",
  "friends",
  "inventory",
  "jobs",
  "legal",
  "login",
  "logout",
  "messages",
  "moderator",
  "p",
  "passport",
  "payments",
  "popout",
  "prime",
  "privacy",
  "products",
  "recap",
  "redeem",
  "search",
  "security",
  "settings",
  "signup",
  "store",
  "subs",
  "subscriptions",
  "team",
  "turbo",
  "u",
  "user",
  "videos",
  "wallet",
]);

const LOGIN = /^[a-z0-9_]{1,25}$/;
const MAX_NAME_CHARS = 100;

/** Un login de chaîne valide (le format de l'interprète) et pas une page de Twitch, sinon `null`. */
export function asLogin(segment: string | undefined): string | null {
  if (segment === undefined) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(segment).toLowerCase();
  } catch {
    return null;
  }
  return LOGIN.test(decoded) && !RESERVED.has(decoded) ? decoded : null;
}

const NONE: Route = { content: "none", login: null, routeKey: null };

/** L'URL d'une page de www.twitch.tv ou de clips.twitch.tv, sans le DOM. */
export function parseRoute(href: string): Route {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return NONE;
  }
  if (url.protocol !== "https:") return NONE;
  const parts = url.pathname.split("/").filter((p) => p !== "");
  const [first, second, third] = parts;

  if (url.hostname === "clips.twitch.tv") {
    // clips.twitch.tv/<slug> ; la chaîne vient du DOM.
    if (first === undefined || first === "embed" || parts.length !== 1) return NONE;
    return { content: "clip", login: null, routeKey: `clip:${first}` };
  }
  if (url.hostname !== "www.twitch.tv") return NONE;

  if (first === "videos" && second !== undefined && /^\d+$/.test(second)) {
    return { content: "vod", login: null, routeKey: `vod:${second}` };
  }
  if (first === "collections" && second !== undefined) {
    // Une collection lit des rediffusions ; la chaîne vient du DOM.
    return { content: "vod", login: null, routeKey: `collection:${second}` };
  }
  if (first === "popout") {
    // /popout/<login>/… : le lecteur détaché d'une chaîne.
    const login = asLogin(second);
    return login === null ? NONE : { content: "live", login, routeKey: `live:${login}` };
  }
  const login = asLogin(first);
  if (login === null) return NONE;
  if (second === "clip" && third !== undefined) {
    return { content: "clip", login, routeKey: `clip:${third}` };
  }
  if ((second === "video" || second === "v") && third !== undefined && /^\d+$/.test(third)) {
    return { content: "vod", login, routeKey: `vod:${third}` };
  }
  // /<login>, /<login>/videos, /<login>/about… : le lecteur de la chaîne est celui du live.
  return { content: "live", login, routeKey: `live:${login}` };
}

function* matches(root: QueryRoot, selectors: readonly string[]): Iterable<ElementLike> {
  for (const selector of selectors) {
    try {
      yield* root.querySelectorAll(selector);
    } catch {
      // Un sélecteur refusé par le navigateur ne doit pas arrêter la mesure.
    }
  }
}

/** Le login d'un lien vers une chaîne (`/<login>` ou `https://www.twitch.tv/<login>`), sinon `null`. */
export function loginFromHref(href: string | null): string | null {
  if (href === null) return null;
  let url: URL;
  try {
    url = new URL(href, "https://www.twitch.tv/");
  } catch {
    return null;
  }
  if (url.hostname !== "www.twitch.tv") return null;
  const parts = url.pathname.split("/").filter((p) => p !== "");
  return parts.length === 1 ? asLogin(parts[0]) : null;
}

function firstLogin(root: QueryRoot, selectors: readonly string[]): string | null {
  for (const element of matches(root, selectors)) {
    const login = loginFromHref(element.getAttribute("href"));
    if (login !== null) return login;
  }
  return null;
}

/** Un texte court d'une ligne, ou `null`. */
export function cleanText(text: string | null): string | null {
  const clean = (text ?? "").replace(/\s+/g, " ").trim();
  return clean === "" || clean.length > MAX_NAME_CHARS ? null : clean;
}

/** Le nom affiché, seulement s'il est le login à la casse près : jamais un titre pris par erreur. */
function displayName(root: QueryRoot, login: string): string | null {
  for (const element of matches(root, SELECTORS.channelName)) {
    const name = cleanText(element.textContent);
    if (name !== null && name.toLowerCase() === login) return name;
  }
  return null;
}

/** La catégorie, seulement depuis un lien vers l'annuaire. */
function category(root: QueryRoot): string | null {
  for (const element of matches(root, SELECTORS.category)) {
    const href = element.getAttribute("href") ?? "";
    if (!/^(https:\/\/www\.twitch\.tv)?\/directory\//.test(href)) continue;
    const name = cleanText(element.textContent);
    if (name !== null) return name;
  }
  return null;
}

/**
 * Ce qui se regarde sur cette page, ou `null` si la page n'est pas une page de lecture. La chaîne
 * vient de l'URL quand elle y est, du DOM sinon (rediffusion, clip de clips.twitch.tv), ou reste `null`.
 */
export function readContext(href: string, root: QueryRoot): WatchContext | null {
  const route = parseRoute(href);
  if (route.content === "none" || route.routeKey === null) return null;
  const channel = route.login ?? firstLogin(root, SELECTORS.channelLink);
  return {
    channel,
    channelName: channel === null ? null : displayName(root, channel),
    content: route.content,
    category: category(root),
    routeKey: route.routeKey,
  };
}

/** Un `<video>` dans le mini-lecteur, sur une page qui n'est pas de lecture : sa chaîne, en live. */
export function miniPlayerContext(video: VideoLike): WatchContext | null {
  for (const selector of SELECTORS.miniPlayer) {
    let container: QueryRoot | null;
    try {
      container = video.closest(selector);
    } catch {
      continue;
    }
    if (container === null) continue;
    const channel = firstLogin(container, SELECTORS.miniPlayerChannelLink);
    if (channel !== null) {
      return { channel, channelName: null, content: "live", category: null, routeKey: `live:${channel}` };
    }
  }
  return null;
}

/**
 * Le contexte d'un `<video>`, qui garde sa chaîne d'un passage à l'autre (mini-lecteur) :
 * - page de lecture : le contexte de la page ; sur la même route, ce que le DOM n'a pas encore
 *   affiché (chaîne, nom, catégorie) reprend la valeur déjà connue ;
 * - autre page : le contexte déjà lié à ce `<video>`, ou celui du mini-lecteur, ou `null` (une vidéo
 *   jamais vue sur une page de lecture ne compte pas : l'aperçu de l'accueil ou d'un survol).
 */
export function bindContext(
  previous: WatchContext | null,
  page: WatchContext | null,
  miniPlayer: WatchContext | null,
): WatchContext | null {
  if (page === null) return previous ?? miniPlayer;
  if (previous === null || previous.routeKey !== page.routeKey) return page;
  const channel = page.channel ?? previous.channel;
  return {
    channel,
    channelName:
      page.channel === null || page.channel === previous.channel
        ? (page.channelName ?? previous.channelName)
        : page.channelName,
    content: page.content,
    category: page.category ?? previous.category,
    routeKey: page.routeKey,
  };
}

/** Deux contextes qui découpent un segment : la chaîne ou le contenu change. */
export function sameWatch(a: WatchContext, b: WatchContext): boolean {
  return a.channel === b.channel && a.content === b.content && a.routeKey === b.routeKey;
}
