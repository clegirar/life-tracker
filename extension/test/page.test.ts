import assert from "node:assert/strict";
import { test } from "node:test";

import {
  asLogin,
  bindContext,
  loginFromHref,
  miniPlayerContext,
  parseRoute,
  readContext,
  SELECTORS,
  type ElementLike,
  type QueryRoot,
  type WatchContext,
} from "../src/lib/page.js";

/** Un faux DOM : chaque sélecteur rend ses éléments. */
function dom(bySelector: Record<string, Array<{ href?: string; text?: string }>>): QueryRoot {
  return {
    querySelectorAll(selector: string): ElementLike[] {
      return (bySelector[selector] ?? []).map((e) => ({
        getAttribute: (name: string) => (name === "href" ? (e.href ?? null) : null),
        textContent: e.text ?? null,
      }));
    },
  };
}
const EMPTY = dom({});

test("l'URL donne la chaîne et le contenu", () => {
  assert.deepEqual(parseRoute("https://www.twitch.tv/Chaine_Fictive"), {
    content: "live",
    login: "chaine_fictive",
    routeKey: "live:chaine_fictive",
  });
  assert.deepEqual(parseRoute("https://www.twitch.tv/chaine_fictive/videos?filter=archives"), {
    content: "live",
    login: "chaine_fictive",
    routeKey: "live:chaine_fictive",
  });
  assert.deepEqual(parseRoute("https://www.twitch.tv/videos/123456789?t=1h2m"), {
    content: "vod",
    login: null,
    routeKey: "vod:123456789",
  });
  assert.deepEqual(parseRoute("https://www.twitch.tv/chaine_fictive/clip/ClipFictifAbc-123"), {
    content: "clip",
    login: "chaine_fictive",
    routeKey: "clip:ClipFictifAbc-123",
  });
  assert.deepEqual(parseRoute("https://clips.twitch.tv/ClipFictifAbc-123"), {
    content: "clip",
    login: null,
    routeKey: "clip:ClipFictifAbc-123",
  });
  assert.deepEqual(parseRoute("https://www.twitch.tv/popout/chaine_fictive/chat?popout="), {
    content: "live",
    login: "chaine_fictive",
    routeKey: "live:chaine_fictive",
  });
});

test("les pages de Twitch ne sont pas des chaînes", () => {
  for (const path of [
    "",
    "directory",
    "directory/category/just-chatting",
    "videos",
    "settings/profile",
    "search?term=x",
    "subscriptions",
    "inventory",
    "wallet",
    "drops/campaigns",
    "downloads",
    "p/fr/legal/terms-of-service",
    "jobs",
    "turbo",
    "friends",
    "messages",
    "popout",
    "popout/directory/chat",
    "moderator/chaine_fictive",
    "u/chaine_fictive/dashboard",
  ]) {
    assert.equal(parseRoute(`https://www.twitch.tv/${path}`).content, "none", path);
  }
  assert.equal(parseRoute("https://clips.twitch.tv/embed?clip=x").content, "none");
  assert.equal(parseRoute("http://www.twitch.tv/chaine_fictive").content, "none");
  assert.equal(parseRoute("https://m.twitch.tv/chaine_fictive").content, "none");
  assert.equal(parseRoute("pas une url").content, "none");
});

test("un login suit le format de l'interprète", () => {
  assert.equal(asLogin("Chaine_Fictive"), "chaine_fictive");
  assert.equal(asLogin("a".repeat(25)), "a".repeat(25));
  assert.equal(asLogin("a".repeat(26)), null);
  assert.equal(asLogin("avec-tiret"), null);
  assert.equal(asLogin("%E2%9C%93"), null);
  assert.equal(asLogin("%"), null);
  assert.equal(asLogin("directory"), null);
  assert.equal(loginFromHref("/chaine_fictive"), "chaine_fictive");
  assert.equal(loginFromHref("https://www.twitch.tv/chaine_fictive"), "chaine_fictive");
  assert.equal(loginFromHref("/chaine_fictive/videos"), null);
  assert.equal(loginFromHref("/directory"), null);
  assert.equal(loginFromHref("https://exemple.invalid/chaine_fictive"), null);
  assert.equal(loginFromHref(null), null);
});

test("un live : la chaîne de l'URL, le nom et la catégorie du DOM", () => {
  const root = dom({
    [SELECTORS.channelName[0]]: [{ text: "  Chaine_Fictive " }],
    [SELECTORS.category[0]]: [{ href: "/directory/category/just-chatting", text: "Just Chatting" }],
  });
  assert.deepEqual(readContext("https://www.twitch.tv/chaine_fictive", root), {
    channel: "chaine_fictive",
    channelName: "Chaine_Fictive",
    content: "live",
    category: "Just Chatting",
    routeKey: "live:chaine_fictive",
  });
});

test("un nom affiché qui n'est pas le login est ignoré (jamais un titre)", () => {
  const root = dom({
    [SELECTORS.channelName[0]]: [{ text: "Titre du stream fictif !" }],
    [SELECTORS.channelName[2]]: [{ text: "CHAINE_FICTIVE" }],
  });
  assert.equal(readContext("https://www.twitch.tv/chaine_fictive", root)?.channelName, "CHAINE_FICTIVE");
  assert.equal(
    readContext("https://www.twitch.tv/chaine_fictive", dom({ "h1.tw-title": [{ text: "Autre chose" }] }))?.channelName,
    null,
  );
});

test("la catégorie ne vient que d'un lien vers l'annuaire", () => {
  const root = dom({ [SELECTORS.category[0]]: [{ href: "/autre", text: "Pas une catégorie" }] });
  assert.equal(readContext("https://www.twitch.tv/chaine_fictive", root)?.category, null);
});

test("une rediffusion ou un clip : la chaîne vient du DOM, sinon null", () => {
  const root = dom({
    [SELECTORS.channelLink[0]]: [{ href: "/directory/category/x" }, { href: "/chaine_fictive" }],
  });
  assert.equal(readContext("https://www.twitch.tv/videos/42", root)?.channel, "chaine_fictive");
  assert.equal(readContext("https://www.twitch.tv/videos/42", EMPTY)?.channel, null);
  assert.equal(readContext("https://www.twitch.tv/videos/42", EMPTY)?.content, "vod");
  const clip = dom({ [SELECTORS.channelLink[2]]: [{ href: "https://www.twitch.tv/chaine_fictive" }] });
  assert.equal(readContext("https://clips.twitch.tv/ClipFictif", clip)?.channel, "chaine_fictive");
  assert.equal(readContext("https://www.twitch.tv/directory", root), null);
});

test("un sélecteur refusé par le navigateur n'arrête rien", () => {
  const root: QueryRoot = {
    querySelectorAll() {
      throw new SyntaxError("sélecteur invalide");
    },
  };
  assert.equal(readContext("https://www.twitch.tv/chaine_fictive", root)?.channel, "chaine_fictive");
});

const live = (login: string, extra: Partial<WatchContext> = {}): WatchContext => ({
  channel: login,
  channelName: null,
  content: "live",
  category: null,
  routeKey: `live:${login}`,
  ...extra,
});

test("le mini-lecteur garde la chaîne de sa vidéo", () => {
  const watched = live("chaine_fictive", { category: "Just Chatting" });
  // Sur l'annuaire, la vidéo déjà liée garde sa chaîne.
  assert.deepEqual(bindContext(watched, null, null), watched);
  // Une vidéo jamais vue sur une page de lecture (aperçu de l'accueil) ne compte pas.
  assert.equal(bindContext(null, null, null), null);
  // Sinon, le mini-lecteur, s'il dit sa chaîne.
  assert.deepEqual(bindContext(null, null, live("autre_chaine")), live("autre_chaine"));
  // Sur une autre chaîne, la vidéo change de chaîne.
  assert.deepEqual(bindContext(watched, live("autre_chaine"), null), live("autre_chaine"));
});

test("sur la même page, ce que le DOM n'affiche plus reste connu", () => {
  const vod: WatchContext = { channel: "chaine_fictive", channelName: "Chaine_Fictive", content: "vod", category: "Art", routeKey: "vod:42" };
  const later: WatchContext = { channel: null, channelName: null, content: "vod", category: null, routeKey: "vod:42" };
  assert.deepEqual(bindContext(vod, later, null), vod);
  const other: WatchContext = { ...later, routeKey: "vod:43" };
  assert.deepEqual(bindContext(vod, other, null), other);
});

test("le mini-lecteur : un lien de chaîne dans son conteneur", () => {
  const container = dom({ [SELECTORS.miniPlayerChannelLink[0]]: [{ href: "/chaine_fictive" }] });
  const inMini = { closest: (s: string) => (s === SELECTORS.miniPlayer[0] ? container : null) };
  assert.deepEqual(miniPlayerContext(inMini), live("chaine_fictive"));
  assert.equal(miniPlayerContext({ closest: () => null }), null);
});
