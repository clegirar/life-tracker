// De bout en bout : le flux OAuth complet, puis les outils appelés par le client officiel du SDK, contre
// le vrai backend et un PostgreSQL jetable (mcp/test.sh). Les quatre questions sur la musique de
// l'architecture, et « à quelle heure je prends mon premier café en moyenne ».

import assert from 'node:assert/strict';
import { after, before, describe, test } from 'node:test';

import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';

import { type Harness, login, startServer } from './helpers.js';

const backendUrl = process.env.LT_MCP_TEST_BACKEND_URL;
const serviceToken = process.env.LT_MCP_TEST_SERVICE_TOKEN ?? '';
const iosToken = process.env.LT_MCP_TEST_IOS_TOKEN ?? '';
const lastfmToken = process.env.LT_MCP_TEST_LASTFM_TOKEN ?? '';
const garminToken = process.env.LT_MCP_TEST_GARMIN_TOKEN ?? '';
const twitchToken = process.env.LT_MCP_TEST_TWITCH_TOKEN ?? '';
const steamToken = process.env.LT_MCP_TEST_STEAM_TOKEN ?? '';
const skip = backendUrl === undefined ? 'sans backend : lancer mcp/test.sh' : false;

const EXPECTED_TOOLS = [
  'completeness',
  'cumulative_duration',
  'day_summary',
  'discoveries',
  'discovery',
  'inventory',
  'last_time',
  'occurrences',
  'route',
  'search',
  'top',
];

function cup(key: string, at: string) {
  return { key, kind: 'coffee.cup', schema_version: 1, occurred_at: at, payload: {} };
}

function scrobble(at: string, artist: string, track: string, album: string) {
  return {
    key: `${at}|${artist}|${track}`,
    kind: 'lastfm.scrobble',
    schema_version: 1,
    occurred_at: at,
    payload: { artist: { mbid: '', '#text': artist }, album: { mbid: '', '#text': album }, name: track, date: { uts: '0' } },
  };
}

// Garmin, tel que l'envoie le connecteur : listes blanches, coordonnées et lieu fictifs.
const garmin = (key: string, kind: string, at: string, payload: Record<string, unknown>) => ({ key, kind, schema_version: 1, occurred_at: at, payload });
const GARMIN = [
  garmin('summary:2026-10-15:a', 'garmin.daily_summary', '2026-10-15T12:00:00+02:00', {
    calendar_date: '2026-10-15', steps: 8412, distance_m: 6123, calories_total: 2350, calories_active: 512, calories_resting: 1838,
  }),
  garmin('sleep:2026-10-15:a', 'garmin.sleep', '2026-10-15T12:00:00+02:00', {
    calendar_date: '2026-10-15', start: '2026-10-14T21:41:00Z', end: '2026-10-15T04:54:00Z', sleep_seconds: 25_980,
    deep_seconds: 4_920, light_seconds: 14_400, rem_seconds: 6_660, awake_seconds: 660,
    heart_rate: [[0, 58], [120, 47], [240, 52], [360, 54]],
  }),
  garmin('activity:7:a', 'garmin.activity', '2026-10-15T07:30:00+02:00', {
    activity_id: 7, type: 'running', start: '2026-10-15T05:30:00Z', duration_s: 1802, moving_s: 1780, distance_m: 5012.3,
    avg_speed_mps: 2.78, avg_hr: 151, max_hr: 176, calories: 389, start_lat: 43.212345, start_lon: 5.432109,
    end_lat: 43.222345, end_lon: 5.442109, location_name: 'Ville Fictive',
  }),
  garmin('track:7:a', 'garmin.activity_track', '2026-10-15T07:30:00+02:00', {
    activity_id: 7, points: [[43.212345, 5.432109, 0], [43.218345, 5.438109, 600], [43.222345, 5.442109, 1200]],
  }),
];

// Twitch, tel que l'envoie l'extension : des segments de lecture. Le 16 oct. (UTC+2), alpha de 20:00
// à 21:00 et beta de 20:30 à 21:30 : 90 min passées sur Twitch, 60 pour chaque chaîne.
const segment = (key: string, at: string, channel: string, minutes: number) => ({
  key,
  kind: 'twitch.watch',
  schema_version: 1,
  occurred_at: at,
  payload: { channel, channel_name: channel === 'alpha' ? 'Alpha' : 'Beta', content: 'live', duration_ms: minutes * 60_000, category: null },
});
const TWITCH = [
  segment('t-a1', '2026-10-16T20:00:00+02:00', 'alpha', 20),
  segment('t-a2', '2026-10-16T20:20:00+02:00', 'alpha', 20),
  segment('t-a3', '2026-10-16T20:40:00+02:00', 'alpha', 20),
  segment('t-b1', '2026-10-16T20:30:00+02:00', 'beta', 30),
  segment('t-b2', '2026-10-16T21:00:00+02:00', 'beta', 30),
];

// Steam, tel que l'envoie le collecteur : des relevés du compteur de chaque jeu, avec le précédent. Le
// 17 oct. (UTC+2), le point de départ à 20:00 (120 min), puis 160 min à 21:00, la partie finie à 20:55 :
// 40 min de 20:15 à 20:55. Un succès à 20:50.
const steamReading = (minutes: number, at: string, lastPlayed: string | null, previous: { minutes: number; at: string } | null) => ({
  key: `playtime:4242:${minutes}`,
  kind: 'steam.playtime',
  schema_version: 1,
  occurred_at: at,
  payload: {
    appid: 4242,
    name: 'Jeu Fictif',
    playtime_forever: minutes,
    playtime_disconnected: 0,
    rtime_last_played: lastPlayed === null ? null : Date.parse(lastPlayed) / 1000,
    previous: previous && { playtime_forever: previous.minutes, observed_at: previous.at },
  },
});
const STEAM = [
  steamReading(120, '2026-10-17T20:00:00+02:00', null, null),
  steamReading(160, '2026-10-17T21:00:00+02:00', '2026-10-17T20:55:00+02:00', { minutes: 120, at: '2026-10-17T20:00:00+02:00' }),
  {
    key: 'achievement:4242:ACH_FICTIF',
    kind: 'steam.achievement',
    schema_version: 1,
    occurred_at: '2026-10-17T20:50:00+02:00',
    payload: { appid: 4242, game_name: 'Jeu Fictif', apiname: 'ACH_FICTIF', name: 'Succès fictif' },
  },
];

async function ingest(token: string, records: unknown[]): Promise<void> {
  const response = await fetch(`${backendUrl}/v1/records`, {
    method: 'POST',
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ records }),
  });
  assert.equal(response.status, 200);
  const body = (await response.json()) as { results: { status: string }[] };
  for (const result of body.results) {
    assert.ok(['accepted', 'duplicate'].includes(result.status), JSON.stringify(body));
  }
}

/** Le worker est asynchrone : attend que l'inventaire voie tout. */
async function settled(): Promise<void> {
  for (let i = 0; i < 300; i += 1) {
    const response = await fetch(`${backendUrl}/v1/inventory`, { headers: { authorization: `Bearer ${serviceToken}` } });
    const body = (await response.json()) as { metrics: { metric: string; events: number }[] };
    const events = (name: string) => body.metrics.find((m) => m.metric === name)?.events ?? 0;
    if (
      events('coffee') === 4 &&
      events('music.listen') === 5 &&
      events('music.track_duration') === 1 &&
      events('workout') === 1 &&
      events('workout.track') === 1 &&
      events('sleep') === 1 &&
      events('stream.watch') === 5 &&
      events('game.play') === 1 &&
      events('game.achievement') === 1
    ) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error('le worker n’a pas interprété le jeu de données');
}

type ToolResult = { content: { type: string; text?: string }[]; isError?: boolean };

function parse(result: ToolResult): Record<string, unknown> {
  assert.ok(!result.isError, JSON.stringify(result));
  const text = result.content[0]?.text ?? '';
  assert.ok(text.length < 100_000);
  return JSON.parse(text) as Record<string, unknown>;
}

describe('de bout en bout, contre le vrai backend', { skip }, () => {
  let h: Harness;
  let client: Client;
  let modern: Client;

  before(async () => {
    // 08:10, 14:00, 09:20 et 07:50 à Paris (heure d'été).
    await ingest(iosToken, [
      cup('e2e-1', '2026-10-12T06:10:00Z'),
      cup('e2e-2', '2026-10-12T12:00:00Z'),
      cup('e2e-3', '2026-10-13T07:20:00Z'),
      cup('e2e-4', '2026-10-14T05:50:00Z'),
    ]);
    await ingest(lastfmToken, [
      scrobble('2026-10-12T18:00:00Z', 'Daft Punk', 'Veridis Quo', 'Discovery'),
      scrobble('2026-10-13T18:00:00Z', 'Daft Punk', 'Veridis Quo', 'Discovery'),
      scrobble('2026-10-14T18:00:00Z', 'Daft Punk', 'Veridis Quo (Remastered)', 'Discovery'),
      scrobble('2026-10-13T19:00:00Z', 'Justice', 'Genesis', 'Cross'),
      // Hors des périodes d'octobre interrogées ailleurs : une écoute à trois artistes crédités.
      scrobble('2026-09-20T18:00:00Z', 'Zek, Jungle Jack & Caballero', 'T', 'GRAND LUXXXE Vol.1'),
      {
        key: 'info-veridis',
        kind: 'lastfm.track_info',
        schema_version: 1,
        occurred_at: '2026-10-12T18:01:00Z',
        payload: { artist: 'Daft Punk', track: 'Veridis Quo', info: { track: { duration: '345000' } } },
      },
    ]);
    await ingest(garminToken, GARMIN);
    await ingest(twitchToken, TWITCH);
    await ingest(steamToken, STEAM);
    await settled();
    h = await startServer({ backendUrl, serviceToken });
    const tokens = await login(h);
    const headers = { authorization: `Bearer ${tokens.access_token}` };
    client = new Client({ name: 'e2e', version: '1.0.0' });
    await client.connect(new StreamableHTTPClientTransport(new URL(`${h.base}/mcp`), { requestInit: { headers } }));
    modern = new Client({ name: 'e2e-modern', version: '1.0.0' }, { versionNegotiation: { mode: 'auto' } });
    await modern.connect(new StreamableHTTPClientTransport(new URL(`${h.base}/mcp`), { requestInit: { headers } }));
  });

  after(async () => {
    await client?.close();
    await modern?.close();
    await h?.close();
  });

  const call = async (name: string, args: Record<string, unknown> = {}) =>
    parse((await client.callTool({ name, arguments: args })) as ToolResult);

  test('les outils sont exactement ceux de l’architecture, tous en lecture seule', async () => {
    for (const c of [client, modern]) {
      const { tools } = await c.listTools();
      assert.deepEqual(tools.map((t) => t.name).sort(), EXPECTED_TOOLS);
      for (const tool of tools) {
        assert.equal(tool.annotations?.readOnlyHint, true, tool.name);
        assert.equal(tool.annotations?.destructiveHint, false, tool.name);
      }
    }
  });

  test('l’inventaire dit quoi demander', async () => {
    const inventory = await call('inventory');
    const metrics = inventory.metrics as { metric: string; tools: string[]; events: number }[];
    const listen = metrics.find((m) => m.metric === 'music.listen');
    assert.ok(listen?.tools.includes('cumulative_duration'));
    assert.ok(listen?.tools.includes('discovery'));
    const coffee = metrics.find((m) => m.metric === 'coffee');
    assert.ok(coffee && !coffee.tools.includes('cumulative_duration'));
    assert.equal(coffee.events, 4);
    assert.ok(!metrics.some((m) => m.metric === 'music.known'), 'les déclarations ne sont pas des métriques à interroger');
  });

  test('à quelle heure je prends mon premier café en moyenne', async () => {
    const result = await call('occurrences', { metric: 'coffee', from: '2026-10-12', to: '2026-10-18', form: 'first_last' });
    const days = result.days as { date: string; first: string }[];
    assert.deepEqual(
      days.map((d) => [d.date, d.first]),
      [
        ['2026-10-12', '08:10'],
        ['2026-10-13', '09:20'],
        ['2026-10-14', '07:50'],
      ],
    );
    const minutes = days.map((d) => Number(d.first.slice(0, 2)) * 60 + Number(d.first.slice(3)));
    const average = minutes.reduce((a, b) => a + b, 0) / minutes.length;
    assert.equal(`${Math.floor(average / 60)}:${String(Math.floor(average % 60)).padStart(2, '0')}`, '8:26');
  });

  test('combien de fois j’ai écouté tel titre ces derniers jours', async () => {
    const found = (await call('search', { query: 'veridis quo', metric: 'music.listen', level: 'track' })) as unknown;
    const first = (found as { key: string; name: string }[])[0];
    assert.equal(first?.key, 'daft punk\tveridis quo');
    const result = await call('occurrences', {
      metric: 'music.listen',
      level: 'track',
      key: first.key,
      from: '2026-10-08',
      to: '2026-10-14',
      form: 'daily',
    });
    assert.equal(result.total, 3);
  });

  test('combien de minutes d’écoute cette semaine', async () => {
    const result = await call('cumulative_duration', { metric: 'music.listen', from: '2026-10-12', to: '2026-10-18', group: 'week' });
    // 3 × 345 s pour Veridis Quo ; Genesis n'a pas de durée connue.
    assert.equal(result.total_minutes, 17);
    assert.equal(result.events_without_duration, 1);
    assert.deepEqual((result.periods as { period: string }[]).map((p) => p.period), ['2026-W42']);
  });

  test('quand ai-je découvert tel artiste', async () => {
    const result = await call('discovery', { metric: 'music.listen', level: 'artist', key: 'justice' });
    assert.equal(result.discovered_at, '2026-10-13 21:00');
    assert.equal(result.already_known, null);
  });

  test('depuis quand je n’ai pas écouté tel album', async () => {
    const found = (await call('search', { query: 'discovery', level: 'album' })) as unknown;
    const album = (found as { key: string; context: Record<string, string> }[])[0];
    assert.equal(album?.context.artist, 'Daft Punk');
    const result = await call('last_time', { metric: 'music.listen', level: 'album', key: album.key });
    assert.equal(result.last_occurrence, '2026-10-14 20:00');
    assert.equal(result.occurrences, 3);
  });

  test('classement, découvertes, résumé d’une journée et complétude', async () => {
    const top = await call('top', { metric: 'music.listen', level: 'artist', from: '2026-10-01', to: '2026-10-31' });
    assert.equal((top.items as { name: string }[])[0]?.name, 'Daft Punk');
    const discoveries = await call('discoveries', { metric: 'music.listen', level: 'track', from: '2026-10-13', to: '2026-10-13' });
    assert.deepEqual((discoveries.items as { name: string }[]).map((d) => d.name), ['Genesis']);
    const day = await call('day_summary', { date: '2026-10-13' });
    const metrics = day.metrics as { metric: string; value: number | null; times?: string[]; minutes?: number }[];
    assert.deepEqual(metrics.find((m) => m.metric === 'coffee')?.times, ['09:20']);
    assert.equal(metrics.find((m) => m.metric === 'music.listen')?.value, 2);
    assert.equal(metrics.find((m) => m.metric === 'music.listen')?.minutes, 6);
    const completeness = await call('completeness', { from: '2026-10-01', to: '2026-10-31' });
    const coffee = (completeness.metrics as { metric: string; days_without_data: string[] }[]).find((m) => m.metric === 'coffee');
    assert.deepEqual(coffee?.days_without_data, ['2026-10-01..2026-10-11']);
  });

  test('combien d’artistes uniques : une écoute à trois artistes en compte trois', async () => {
    const artists = await call('top', { metric: 'music.listen', level: 'artist', from: '2026-09-20', to: '2026-09-20' });
    assert.equal(artists.distinct_items, 3);
    assert.equal(artists.occurrences_with_item, 1);
    const tracks = await call('top', { metric: 'music.listen', level: 'track', from: '2026-09-20', to: '2026-09-20' });
    assert.equal(tracks.distinct_items, 1);
    const track = (tracks.items as { context: Record<string, string> }[])[0];
    assert.equal(track?.context.artist, 'Zek, Jungle Jack & Caballero');
  });

  test('une erreur de paramètre se dit à Claude, sans planter', async () => {
    const result = (await client.callTool({
      name: 'occurrences',
      arguments: { metric: 'inconnue', from: '2026-10-01', to: '2026-10-02' },
    })) as ToolResult;
    assert.equal(result.isError, true);
    assert.match(result.content[0]?.text ?? '', /Invalid parameter: metric/);
    const tooLong = (await client.callTool({ name: 'completeness', arguments: { from: '2025-01-01', to: '2026-10-31' } })) as ToolResult;
    assert.equal(tooLong.isError, true);
  });

  test('le client moderne (2026-07-28) obtient les mêmes réponses', async () => {
    const result = parse(
      (await modern.callTool({
        name: 'occurrences',
        arguments: { metric: 'coffee', from: '2026-10-12', to: '2026-10-12', form: 'first_last' },
      })) as ToolResult,
    );
    assert.equal((result.days as { first: string }[])[0]?.first, '08:10');
  });

  test('Twitch : deux streams à la fois comptent une fois, chaque chaîne en entier, le classement par minutes', async () => {
    const inventory = await call('inventory');
    const stream = (inventory.metrics as { metric: string; shape: string; tools: string[]; events: number }[]).find(
      (m) => m.metric === 'stream.watch',
    );
    assert.equal(stream?.shape, 'duration');
    assert.equal(stream?.events, 5);
    for (const tool of ['cumulative_duration', 'top', 'discoveries', 'search']) {
      assert.ok(stream?.tools.includes(tool), tool);
    }
    const total = await call('cumulative_duration', { metric: 'stream.watch', from: '2026-10-16', to: '2026-10-16' });
    assert.equal(total.total_minutes, 90, 'l’union, pas la somme');
    const alpha = await call('cumulative_duration', {
      metric: 'stream.watch',
      from: '2026-10-01',
      to: '2026-10-31',
      group: 'month',
      level: 'channel',
      key: 'alpha',
    });
    assert.equal(alpha.total_minutes, 60);
    assert.deepEqual((alpha.periods as { period: string }[]).map((p) => p.period), ['2026-10']);
    const day = await call('day_summary', { date: '2026-10-16' });
    type Ranked = { key: string; name: string; minutes: number; occurrences: number };
    type Entry = { metric: string; value: number | null; minutes?: number; top?: Record<string, { items: Ranked[] }> };
    const entry = (day.metrics as Entry[]).find((m) => m.metric === 'stream.watch');
    assert.equal(entry?.value, 90);
    assert.equal(entry?.minutes, 90);
    assert.deepEqual(
      entry?.top?.channel?.items.map((i) => [i.key, i.minutes, i.occurrences]),
      [
        ['alpha', 60, 3],
        ['beta', 60, 2],
      ],
    );
    const discoveries = await call('discoveries', { metric: 'stream.watch', level: 'channel', from: '2026-10-16', to: '2026-10-16' });
    assert.deepEqual((discoveries.items as { name: string }[]).map((d) => d.name), ['Alpha', 'Beta']);
  });

  test('Steam : le temps de jeu déduit des relevés, le classement par minutes, les succès, pas de fausse découverte', async () => {
    const inventory = await call('inventory');
    type Inventoried = { metric: string; shape: string; tools: string[]; events: number };
    const play = (inventory.metrics as Inventoried[]).find((m) => m.metric === 'game.play');
    assert.equal(play?.shape, 'duration');
    assert.equal(play?.events, 1);
    for (const tool of ['cumulative_duration', 'top', 'search']) {
      assert.ok(play?.tools.includes(tool), tool);
    }
    const total = await call('cumulative_duration', { metric: 'game.play', from: '2026-10-17', to: '2026-10-17' });
    assert.equal(total.total_minutes, 40, 'l’écart entre les deux relevés');
    const top = await call('top', { metric: 'game.play', level: 'game', from: '2026-10-01', to: '2026-10-31' });
    assert.deepEqual(
      (top.items as { key: string; name: string; minutes: number }[]).map((i) => [i.key, i.name, i.minutes]),
      [['4242', 'Jeu Fictif', 40]],
    );
    const achievements = await call('occurrences', { metric: 'game.achievement', from: '2026-10-17', to: '2026-10-17', form: 'first_last' });
    assert.equal((achievements.days as { first: string }[])[0]?.first, '20:50');
    // Sans valeur du jour, les succès n'ont ni case du calendrier ni place dans `day_summary`.
    const achievement = (inventory.metrics as Inventoried[]).find((m) => m.metric === 'game.achievement');
    assert.ok(achievement?.tools.includes('occurrences'));
    assert.ok(!achievement?.tools.includes('day_summary'));
    // Le premier relevé avait déjà 120 min : un jeu joué avant le suivi, déjà connu, jamais une découverte.
    assert.ok(play?.tools.includes('discoveries'));
    const found = await call('discoveries', { metric: 'game.play', level: 'game', from: '2026-10-01', to: '2026-10-31' });
    assert.deepEqual(found.items, []);
    assert.equal(found.marked_known, 1);
  });

  test('Garmin : détails du jour, séances par lieu, parcours coupé à la zone de confidentialité', async () => {
    const outputs: string[] = [];
    const tool = async (name: string, args: Record<string, unknown>) => {
      const result = (await client.callTool({ name, arguments: args })) as ToolResult;
      outputs.push(result.content[0]?.text ?? '');
      return parse(result);
    };
    const day = await tool('day_summary', { date: '2026-10-15' });
    type Occurrence = { id: string; time: string; items: Record<string, string>; value: number; details: Record<string, unknown> };
    type Entry = { metric: string; value: number | null; times?: string[]; details?: Record<string, unknown>; occurrences?: Occurrence[] };
    const metrics = day.metrics as Entry[];
    assert.deepEqual(
      metrics.map((m) => m.metric),
      ['coffee', 'music.listen', 'steps', 'workout', 'sleep', 'calories.burned', 'video.watch', 'stream.watch', 'game.play'],
      'l’ordre d’affichage du registre',
    );
    const steps = metrics.find((m) => m.metric === 'steps');
    assert.equal(steps?.value, 8412);
    assert.equal(steps?.times, undefined, 'une mesure n’a pas d’heure à lister');
    assert.equal(steps?.details?.distance_m, 6123);
    const sleep = metrics.find((m) => m.metric === 'sleep');
    assert.equal(sleep?.value, 433);
    assert.equal(sleep?.details?.rem_seconds, 6_660);
    assert.equal(sleep?.details?.lowest_hr, 47);
    assert.equal(sleep?.details?.avg_hr, 53);
    assert.equal(sleep?.details?.heart_rate, undefined, 'la série reste au journal');
    assert.equal(metrics.find((m) => m.metric === 'calories.burned')?.details?.active, 512);
    const workout = metrics.find((m) => m.metric === 'workout');
    assert.equal(workout?.value, 1);
    const run = workout?.occurrences?.[0];
    assert.equal(run?.time, '07:30');
    assert.deepEqual(run?.items, { activity_type: 'running', place: 'Ville Fictive' });
    assert.equal(run?.value, 5.012);
    assert.equal(run?.details.avg_hr, 151);
    assert.equal(run?.details.moving_s, 1780);
    assert.equal(run?.details.start_lat, null, 'le départ est dans la zone');
    assert.equal(run?.details.start_hidden, true);
    assert.equal(run?.details.end_lat, 43.222345, 'l’arrivée est dehors');
    assert.equal(metrics.find((m) => m.metric === 'workout.track'), undefined, 'une référence ne s’affiche pas');

    // Quel parcours ? Coupé à la zone : le premier point n'y est plus.
    const route = await tool('route', { id: run!.id });
    assert.equal(route.trimmed, true);
    assert.deepEqual(route.segments, [[[43.21835, 5.43811], [43.22235, 5.44211]]]);

    // Dans combien de villes ai-je couru ?
    const places = await tool('top', { metric: 'workout', level: 'place', from: '2026-10-01', to: '2026-10-31' });
    assert.equal(places.distinct_items, 1);
    const inventory = await tool('inventory', {});
    const workoutTools = (inventory.metrics as { metric: string; tools: string[] }[]).find((m) => m.metric === 'workout')?.tools;
    assert.ok(workoutTools?.includes('route'));
    await tool('occurrences', { metric: 'workout', from: '2026-10-15', to: '2026-10-15' });
    await tool('occurrences', { metric: 'workout.track', from: '2026-10-15', to: '2026-10-15' });
    await tool('search', { query: 'ville' });
    const all = outputs.join('');
    assert.match(all, /Ville Fictive/);
    for (const forbidden of ['43.212345', '43.21234', '43.21235', '5.432109', '"points"']) {
      assert.ok(!all.includes(forbidden), `${forbidden} visible par le MCP`);
    }
  });
});
