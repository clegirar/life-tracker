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
      events('sleep') === 1
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

  test('Garmin : mesures du jour, séances par lieu, jamais de coordonnées', async () => {
    const outputs: string[] = [];
    const tool = async (name: string, args: Record<string, unknown>) => {
      const result = (await client.callTool({ name, arguments: args })) as ToolResult;
      outputs.push(result.content[0]?.text ?? '');
      return parse(result);
    };
    const day = await tool('day_summary', { date: '2026-10-15' });
    type Entry = { metric: string; value: number | null; times?: string[]; occurrences?: { time: string; items: Record<string, string>; value: number }[] };
    const metrics = day.metrics as Entry[];
    const steps = metrics.find((m) => m.metric === 'steps');
    assert.equal(steps?.value, 8412);
    assert.equal(steps?.times, undefined, 'une mesure n’a pas d’heure à lister');
    assert.equal(metrics.find((m) => m.metric === 'sleep')?.value, 433);
    const workout = metrics.find((m) => m.metric === 'workout');
    assert.equal(workout?.value, 1);
    assert.deepEqual(workout?.occurrences, [{ time: '07:30', items: { activity_type: 'running', place: 'Ville Fictive' }, value: 5.012 }]);
    assert.equal(metrics.find((m) => m.metric === 'workout.track'), undefined, 'une référence ne s’affiche pas');

    // Dans combien de villes ai-je couru ?
    const places = await tool('top', { metric: 'workout', level: 'place', from: '2026-10-01', to: '2026-10-31' });
    assert.equal(places.distinct_items, 1);
    await tool('occurrences', { metric: 'workout', from: '2026-10-15', to: '2026-10-15' });
    await tool('occurrences', { metric: 'workout.track', from: '2026-10-15', to: '2026-10-15' });
    await tool('inventory', {});
    await tool('search', { query: 'ville' });
    const all = outputs.join('');
    assert.match(all, /Ville Fictive/);
    for (const forbidden of ['43.21', '5.43', 'start_lat', '"points"']) {
      assert.ok(!all.includes(forbidden), `${forbidden} visible par le MCP`);
    }
  });
});
