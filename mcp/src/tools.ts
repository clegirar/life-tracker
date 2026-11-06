// Les outils MCP de l'architecture, tous en lecture seule et génériques : la métrique est un paramètre,
// et le registre (outil `inventory`) dit lesquels s'appliquent à chacune. Une fine couche au-dessus de
// l'API de lecture : aucun calcul de données ici, seulement la mise en forme compacte des réponses.

import { McpServer } from '@modelcontextprotocol/server';
import * as z from 'zod';

import { Backend, BackendError } from './backend.js';

/** Claude refuse un résultat d'outil au-delà d'environ 150 000 caractères : on reste bien en dessous. */
export const MAX_RESULT_CHARS = 100_000;
const MAX_COMPLETENESS_DAYS = 366;

type Json = Record<string, unknown>;

interface RegistryMetric {
  metric: string;
  label: string;
  shape: string;
  unit: string | null;
  domain: string;
  daily_aggregation: string;
  item_levels: string[];
  discovery: boolean;
  applies_to: string | null;
  duration_from: string | null;
  freshness_threshold_seconds: number | null;
  description: string | null;
  shared_details: string[];
  events: number;
  first_at: string | null;
  last_at: string | null;
  sources: { source: string; events: number; first_at: string; last_at: string }[];
}

interface Inventory {
  timezone: string;
  today: string;
  launch_date: string | null;
  metrics: RegistryMetric[];
}

interface MonthDay {
  date: string;
  values: Record<string, number | null>;
  errors: number;
  pending: string[];
  to_check: string[];
}

interface Month {
  month: string;
  today: string;
  days: MonthDay[];
}

interface ItemInfo {
  name: string | null;
  context?: Record<string, string>;
  occurrences: number;
  first_local: string | null;
  last_local: string | null;
  discovery: boolean;
  known_marks: { at: string; source: string }[];
  launch_date: string | null;
}

const READ_ONLY = {
  readOnlyHint: true,
  destructiveHint: false,
  idempotentHint: true,
  openWorldHint: false,
} as const;

const date = z
  .string()
  .regex(/^\d{4}-\d{2}-\d{2}$/)
  .describe('Local date in Europe/Paris, YYYY-MM-DD');
const metric = z.string().min(1).describe('A metric name from `inventory`, e.g. "music.listen" or "coffee"');
const level = z.string().min(1).describe('An item level of the metric, from `inventory` (e.g. "artist", "album", "track")');
const key = z
  .string()
  .min(1)
  .describe('The exact item key returned by `search`, `top` or `discoveries` (keys are normalized, never guess them)');

/** Les outils et ce qui les rend applicables, d'après le registre. */
function toolsFor(m: RegistryMetric): string[] {
  const tools = ['occurrences', 'day_summary', 'completeness'];
  if (m.item_levels.length > 0) {
    tools.push('search', 'top', 'last_time');
  }
  if (m.discovery) {
    tools.push('discovery', 'discoveries');
  }
  if (m.duration_from || m.shape === 'duration') {
    tools.push('cumulative_duration');
  }
  // Des coordonnées dans les détails : la métrique a des parcours (séances).
  if (m.shared_details.some((field) => field.endsWith('_lat'))) {
    tools.push('route');
  }
  return tools;
}

function displayable(m: RegistryMetric): boolean {
  return m.shape !== 'declaration' && m.shape !== 'reference';
}

/** Un résultat compact : JSON sans espaces, borné en taille. */
export function compact(value: unknown): { content: { type: 'text'; text: string }[] } {
  let text = JSON.stringify(value);
  if (text.length > MAX_RESULT_CHARS) {
    text = JSON.stringify({
      truncated: true,
      note: 'Result too large: narrow the period, lower the limit or filter on one item.',
      partial: text.slice(0, MAX_RESULT_CHARS - 300),
    });
  }
  return { content: [{ type: 'text', text }] };
}

function failure(error: unknown): { content: { type: 'text'; text: string }[]; isError: true } {
  let message = 'The life tracker backend is unavailable; try again later.';
  if (error instanceof BackendError) {
    if (error.status === 400) {
      message = `Invalid parameter: ${error.field ?? 'unknown'}. Check names and levels with \`inventory\`, and item keys with \`search\`.`;
    } else if (error.status === 404) {
      message = 'Not found: this item has no occurrence and no "already known" mark. Use `search` to find its exact key.';
    } else if (error.status === 401) {
      message = 'The life tracker refused the MCP service token.';
    } else if (error.status === 403) {
      message = 'Routes and coordinates are withheld: no privacy zone is configured on the server.';
    }
  } else if (error instanceof Error && error.message.startsWith('invalid:')) {
    message = error.message.slice('invalid:'.length).trim();
  }
  return { content: [{ type: 'text', text: message }], isError: true };
}

async function guarded(run: () => Promise<unknown>) {
  try {
    return compact(await run());
  } catch (error) {
    return failure(error);
  }
}

function addDays(day: string, days: number): string {
  const d = new Date(`${day}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

function daysBetween(from: string, to: string): number {
  return Math.round((Date.parse(`${to}T00:00:00Z`) - Date.parse(`${from}T00:00:00Z`)) / 86_400_000);
}

/** Des dates triées en plages compactes : « 2026-01-01..2026-01-31 ». */
export function ranges(days: string[]): string[] {
  const out: string[] = [];
  let start: string | undefined;
  let end: string | undefined;
  for (const day of days) {
    if (end !== undefined && addDays(end, 1) === day) {
      end = day;
      continue;
    }
    if (start !== undefined && end !== undefined) {
      out.push(start === end ? start : `${start}..${end}`);
    }
    start = day;
    end = day;
  }
  if (start !== undefined && end !== undefined) {
    out.push(start === end ? start : `${start}..${end}`);
  }
  return out;
}

function months(from: string, to: string): string[] {
  const out: string[] = [];
  let current = from.slice(0, 7);
  const last = to.slice(0, 7);
  while (current <= last) {
    out.push(current);
    const [y, m] = current.split('-').map(Number) as [number, number];
    current = m === 12 ? `${y + 1}-01` : `${y}-${String(m + 1).padStart(2, '0')}`;
  }
  return out;
}

export function buildServer(backend: Backend): McpServer {
  const server = new McpServer(
    { name: 'life-tracker', version: '0.1.0' },
    {
      instructions:
        'Read-only access to the life tracker of one person (coffee, music listens, steps, sleep, workouts…). Call `inventory` first: it lists the tracked metrics, their item levels and which tools apply. ' +
        'Coordinates near the person\'s home are hidden by a privacy zone (`*_hidden: true`, routes cut where they enter it): never try to infer what was hidden. ' +
        'Dates are local days in Europe/Paris (YYYY-MM-DD, bounds included) and times are Paris local time. Item keys are normalized: get them from `search`, `top` or `discoveries`. ' +
        'A day without occurrences is not necessarily a day without activity: use `completeness` before concluding.',
    },
  );

  server.registerTool(
    'inventory',
    {
      title: 'Inventory',
      description:
        'What is tracked: metrics with label, unit, shape, item levels, data sources and since when, and which tools apply to each. Call it first.',
      inputSchema: z.object({}),
      annotations: READ_ONLY,
    },
    async () =>
      guarded(async () => {
        const inventory = await backend.get<Inventory>('/v1/inventory');
        return {
          timezone: inventory.timezone,
          today: inventory.today,
          launch_date: inventory.launch_date,
          metrics: inventory.metrics.filter(displayable).map((m) => ({
            metric: m.metric,
            label: m.label,
            unit: m.unit,
            domain: m.domain,
            shape: m.shape,
            daily_aggregation: m.daily_aggregation,
            item_levels: m.item_levels,
            description: m.description,
            events: m.events,
            first_at: m.first_at,
            last_at: m.last_at,
            sources: m.sources.map((s) => ({ source: s.source, events: s.events, first_at: s.first_at, last_at: s.last_at })),
            stale_after_seconds: m.freshness_threshold_seconds,
            tools: toolsFor(m),
          })),
          modifiers: inventory.metrics
            .filter((m) => !displayable(m))
            .map((m) => ({ metric: m.metric, label: m.label, applies_to: m.applies_to, description: m.description })),
        };
      }),
  );

  server.registerTool(
    'search',
    {
      title: 'Search',
      description:
        'Find items (artist, album, track…) from an approximate name. Returns their exact keys, with context (e.g. the artist of a track), number of occurrences and first/last date.',
      inputSchema: z.object({
        query: z.string().min(2).describe('An approximate name, e.g. "daft punk" or "veridis quo"'),
        metric: metric.optional(),
        level: level.optional(),
        limit: z.number().int().min(1).max(50).optional().describe('Default 10'),
      }),
      annotations: READ_ONLY,
    },
    async ({ query, metric: m, level: l, limit }) =>
      guarded(() => backend.get('/v1/search', { q: query, metric: m, level: l, limit })),
  );

  server.registerTool(
    'occurrences',
    {
      title: 'Occurrences',
      description:
        'Occurrences of a metric, or of one item of it, over a period, with their Paris local time. Forms: "list" (each occurrence with its id, items, value when the metric has one, e.g. km of a workout or minutes of sleep, and its details: sleep stages in seconds and lowest and average heart rate during the night (bpm), workout duration, pace, heart rate, calories, place, start/end coordinates outside privacy zones), "daily" (count per day), "first_last" (first and last time of each day, e.g. time of the first coffee). Days absent from the result have no occurrence.',
      inputSchema: z.object({
        metric,
        from: date,
        to: date,
        form: z.enum(['list', 'daily', 'first_last']).optional().describe('Default "list"'),
        level: level.optional().describe('With `key`: only this item'),
        key: key.optional(),
        limit: z.number().int().min(1).max(1000).optional().describe('Form "list" only, default 100'),
      }),
      annotations: READ_ONLY,
    },
    async ({ metric: m, from, to, form, level: l, key: k, limit }) =>
      guarded(async () => {
        const result = await backend.get<Json>('/v1/occurrences', { metric: m, from, to, form, level: l, key: k, limit });
        if (result.form === 'list' && Array.isArray(result.occurrences)) {
          result.occurrences = (result.occurrences as Json[]).map((o) => ({
            id: o.id,
            local: o.local,
            items: o.items,
            value: o.value,
            details: o.details,
          }));
        }
        return result;
      }),
  );

  server.registerTool(
    'cumulative_duration',
    {
      title: 'Cumulative duration',
      description:
        'Time spent over a period, in minutes, by day, ISO week (Monday) or month, for a metric with a duration (e.g. listening minutes, Twitch watching). Occurrences without a known duration are counted apart and not included in the minutes. For a metric of shape "duration" (measured time, e.g. Twitch), each occurrence is a time interval: overlapping time (two streams at once) counts once in the total, and fully for each item when filtered by level and key; a period counts only its own part of an interval that crosses midnight.',
      inputSchema: z.object({
        metric,
        from: date,
        to: date,
        group: z.enum(['day', 'week', 'month']).optional().describe('Default "day"'),
        level: level.optional(),
        key: key.optional(),
      }),
      annotations: READ_ONLY,
    },
    async ({ metric: m, from, to, group, level: l, key: k }) =>
      guarded(() => backend.get('/v1/durations', { metric: m, from, to, group, level: l, key: k })),
  );

  const item = (m: string, l: string, k: string) => backend.get<ItemInfo>('/v1/item', { metric: m, level: l, key: k });

  server.registerTool(
    'discovery',
    {
      title: 'Discovery',
      description:
        'When an item (artist, album, track) was discovered: the first occurrence of a lifetime. If it is marked "already known", it was known before the launch date of the app instead.',
      inputSchema: z.object({ metric, level, key }),
      annotations: READ_ONLY,
    },
    async ({ metric: m, level: l, key: k }) =>
      guarded(async () => {
        const info = await item(m, l, k);
        const known = info.known_marks.length > 0;
        return {
          name: info.name,
          context: info.context,
          first_occurrence: info.first_local,
          occurrences: info.occurrences,
          discovered_at: known || !info.discovery ? null : info.first_local,
          already_known: known
            ? { known_before: info.launch_date ?? 'the start of tracking', marked_at: info.known_marks.map((mark) => mark.at) }
            : null,
        };
      }),
  );

  server.registerTool(
    'last_time',
    {
      title: 'Last time',
      description: 'The last occurrence of an item (e.g. the last listen of an album), with the number of occurrences.',
      inputSchema: z.object({ metric, level, key }),
      annotations: READ_ONLY,
    },
    async ({ metric: m, level: l, key: k }) =>
      guarded(async () => {
        const info = await item(m, l, k);
        return {
          name: info.name,
          context: info.context,
          last_occurrence: info.last_local,
          first_occurrence: info.first_local,
          occurrences: info.occurrences,
        };
      }),
  );

  server.registerTool(
    'top',
    {
      title: 'Top',
      description:
        'The most frequent items of a level over a period (e.g. top artists of the month), with their keys. A listen credited to several artists counts for each of them in items and distinct_items, but once in occurrences_with_item. For a metric of shape "duration" (e.g. Twitch channels), items are ranked by time spent (`minutes`), not by occurrences.',
      inputSchema: z.object({
        metric,
        level,
        from: date,
        to: date,
        limit: z.number().int().min(1).max(200).optional().describe('Default 20'),
      }),
      annotations: READ_ONLY,
    },
    async ({ metric: m, level: l, from, to, limit }) =>
      guarded(() => backend.get('/v1/top', { metric: m, level: l, from, to, limit })),
  );

  server.registerTool(
    'discoveries',
    {
      title: 'Discoveries',
      description:
        'Items of a level heard (or seen) for the first time in a lifetime during a period, excluding items marked "already known" at that level.',
      inputSchema: z.object({
        metric,
        level,
        from: date,
        to: date,
        limit: z.number().int().min(1).max(1000).optional().describe('Default 100'),
      }),
      annotations: READ_ONLY,
    },
    async ({ metric: m, level: l, from, to, limit }) =>
      guarded(() => backend.get('/v1/discoveries', { metric: m, level: l, from, to, limit })),
  );

  server.registerTool(
    'day_summary',
    {
      title: 'Day summary',
      description:
        'Everything the calendar shows for one day: the value of each metric (null means no data that day, never zero; a measure such as steps or sleep minutes is the value of the day), times of counted events, each workout with its items and value, top items, discoveries, minutes (for a metric of shape "duration", the value is already the minutes of the day, overlapping time counted once), and the completeness signals of the day.',
      inputSchema: z.object({ date }),
      annotations: READ_ONLY,
    },
    async ({ date: day }) =>
      guarded(async () => {
        const inventory = await backend.get<Inventory>('/v1/inventory');
        const month = await backend.get<Month>(`/v1/months/${day.slice(0, 7)}`);
        const signals = month.days.find((d) => d.date === day);
        const metrics: Json[] = [];
        for (const m of inventory.metrics.filter(displayable)) {
          const entry: Json = { metric: m.metric, label: m.label, value: signals?.values[m.metric] ?? null };
          if (m.shape === 'measure') {
            // Une mesure du jour (pas, sommeil…) : sa valeur et ses détails, sans heure (midi est une convention).
            if (m.shared_details.length > 0) {
              const list = await backend.get<Json>('/v1/occurrences', { metric: m.metric, from: day, to: day, form: 'list', limit: 10 });
              const last = ((list.occurrences as Json[]) ?? []).at(-1);
              entry.details = last?.details ?? null;
            }
          } else if (m.item_levels.length === 0) {
            const list = await backend.get<Json>('/v1/occurrences', {
              metric: m.metric,
              from: day,
              to: day,
              form: 'list',
              limit: 200,
            });
            entry.times = ((list.occurrences as Json[]) ?? []).map((o) => String(o.local).slice(11));
          } else {
            const top: Json = {};
            for (const l of m.item_levels) {
              top[l] = await backend.get('/v1/top', { metric: m.metric, level: l, from: day, to: day, limit: 10 });
            }
            entry.top = top;
            if (!m.discovery) {
              // Peu d'occurrences par jour (des séances) : chacune, avec son heure, ses éléments et sa valeur.
              const list = await backend.get<Json>('/v1/occurrences', { metric: m.metric, from: day, to: day, form: 'list', limit: 50 });
              entry.occurrences = ((list.occurrences as Json[]) ?? []).map((o) => ({
                id: o.id,
                time: String(o.local).slice(11),
                items: o.items,
                value: o.value,
                details: o.details,
              }));
            }
            if (m.discovery) {
              const found: Json = {};
              for (const l of m.item_levels) {
                found[l] = await backend.get('/v1/discoveries', { metric: m.metric, level: l, from: day, to: day, limit: 20 });
              }
              entry.discoveries = found;
            }
          }
          if (m.duration_from || m.shape === 'duration') {
            const durations = await backend.get<Json>('/v1/durations', { metric: m.metric, from: day, to: day });
            entry.minutes = durations.total_minutes;
            entry.events_without_duration = durations.events_without_duration;
          }
          metrics.push(entry);
        }
        return {
          date: day,
          errors: signals?.errors ?? 0,
          pending_sources: signals?.pending ?? [],
          to_check: signals?.to_check ?? [],
          metrics,
        };
      }),
  );

  // Un outil de plus que l'architecture n'en prévoyait : « quel parcours », « où ai-je couru » échouent
  // sans lui (voir le journal des décisions, 4 nov. 2026). Générique : le parcours d'une occurrence.
  server.registerTool(
    'route',
    {
      title: 'Route',
      description:
        'The route of one occurrence (e.g. a run), from its `id` in `occurrences` or `day_summary`: segments of [lat, lon] points in degrees. Points inside the privacy zone are removed and the route is cut there (`trimmed: true`); segments must never be joined across the gap.',
      inputSchema: z.object({
        id: z.string().regex(/^\d+:\d+$/).describe('The occurrence id, e.g. "1234:0"'),
      }),
      annotations: READ_ONLY,
    },
    async ({ id }) => guarded(() => backend.get('/v1/route', { id })),
  );

  server.registerTool(
    'completeness',
    {
      title: 'Completeness',
      description:
        'Gaps and doubts over a period (at most 366 days), so answers can be nuanced: days without data per metric, days with interpretation errors (incomplete), days pending a silent source, metrics to check (no fresh data), and the health of sources.',
      inputSchema: z.object({ from: date, to: date }),
      annotations: READ_ONLY,
    },
    async ({ from, to }) =>
      guarded(async () => {
        const span = daysBetween(from, to);
        if (span < 0 || span >= MAX_COMPLETENESS_DAYS) {
          throw new Error(`invalid: the period must go forward and last at most ${MAX_COMPLETENESS_DAYS} days.`);
        }
        const health = await backend.get<Json>('/v1/sources/health');
        const days: MonthDay[] = [];
        for (const m of months(from, to)) {
          const month = await backend.get<Month>(`/v1/months/${m}`);
          days.push(...month.days.filter((d) => d.date >= from && d.date <= to && d.date <= month.today));
        }
        const metricNames = [...new Set(days.flatMap((d) => Object.keys(d.values)))].sort();
        return {
          period: { from, to },
          interpretation_errors_total: health.errors,
          sources: health.sources,
          metrics: metricNames.map((name) => ({
            metric: name,
            days_without_data: ranges(days.filter((d) => d.values[name] === null).map((d) => d.date)),
            days_to_check: ranges(days.filter((d) => d.to_check.includes(name)).map((d) => d.date)),
          })),
          incomplete_days: days.filter((d) => d.errors > 0).map((d) => ({ date: d.date, errors: d.errors })),
          pending: [...new Set(days.flatMap((d) => d.pending))].map((source) => ({
            source,
            days: ranges(days.filter((d) => d.pending.includes(source)).map((d) => d.date)),
          })),
        };
      }),
  );

  return server;
}
