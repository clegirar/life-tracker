import { lazy, useMemo, useRef, useState } from 'react'
import type { SourcesHealth } from './api'
import { MapFigure } from './ActivityCard'
import {
  distanceText,
  hasTrace,
  hereText,
  iphoneSilent,
  locationKind,
  mapDrawing,
  mapLabel,
  placeRows,
  placesCount,
  stillText,
  traceLengthM,
  uncoveredText,
  type PlaceRow,
} from './location'
import { MoreButton } from './MusicCard'
import { visibleRows } from './music'
import type { useLocations } from './useLocations'
import { useMinute } from './useMinute'

// Leaflet et ses tuiles ne se chargent qu'à l'approche de la carte, comme pour les séances.
const LocationMap = lazy(() => import('./RouteMap').then((module) => ({ default: module.LocationMap })))

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useLocations>
  health: SourcesHealth | null
}

/** Un lieu : son numéro (celui de l'épingle), ses heures, et sa durée à droite. */
function PlaceLine({ row }: { row: PlaceRow }) {
  return (
    <li className="row visit" data-testid="visit" data-ongoing={row.ongoing || undefined}>
      <span className={row.ongoing ? 'visit-number visit-number-current' : 'visit-number'} aria-hidden="true">
        {row.number}
      </span>
      <div className="row-text visit-text">
        <p className="row-name">
          <span className="visually-hidden">Lieu {row.number}, </span>
          {row.range}
        </p>
        {row.note && <p className="row-detail">{row.note}</p>}
      </div>
      <span className="row-count">{row.duration}</span>
    </li>
  )
}

/**
 * Le lieu où je suis encore, en haut de la carte : « Ici depuis 17 h 14 · 1 h 12 », avec la pastille de
 * son épingle. Le temps passé avance à chaque minute, sans animation. Un composant à part : seul lui se
 * redessine chaque minute, pas la carte.
 */
function HereLine({ row }: { row: PlaceRow }) {
  const now = useMinute()
  const { since, elapsed } = hereText(row.arrival, now)
  return (
    <p className="location-here" data-testid="location-here">
      <span className="visit-number visit-number-current" aria-hidden="true">
        {row.number}
      </span>
      <span className="location-here-text">
        <span className="location-here-since">{since}</span>
        <span className="location-here-elapsed"> · {elapsed}</span>
      </span>
    </p>
  )
}

export function LocationCard({ date, today, state, health }: Props) {
  // Déplié pour un jour : changer de jour replie, même en revenant.
  const [unfoldedOn, setUnfoldedOn] = useState<string | null>(null)
  const placesSection = useRef<HTMLElement>(null)
  const { locations, loadFailed } = state
  const unfolded = unfoldedOn === date

  const kind = locations ? locationKind(locations) : null
  const rows = useMemo(() => (locations ? placeRows(locations, today) : []), [locations, today])
  const here = rows.find((row) => row.ongoing) ?? null
  // Le même objet tant que les données ne changent pas : la carte ne se redessine pas à chaque relecture.
  const drawing = useMemo(() => (locations ? mapDrawing(locations, today) : null), [locations, today])
  const shownRows = visibleRows(rows, unfolded).shown
  const places = locations ? placesCount(locations) : null

  return (
    <section className="card card-location" aria-labelledby="titre-deplacements" data-testid="location">
      <h2 id="titre-deplacements" className="card-title card-title-location">
        Déplacements
      </h2>

      {loadFailed && (
        <div className="inline-failure" role="status">
          <span>Impossible d'actualiser les déplacements.</span>
          <button type="button" className="notice-action" onClick={() => void state.reload()}>
            Réessayer
          </button>
        </div>
      )}
      {iphoneSilent(health) && (
        <p className="inline-note" role="status" data-testid="location-alert">
          <strong>L'iPhone n'envoie plus sa position.</strong> Des déplacements de ce jour peuvent manquer.
        </p>
      )}

      {!locations && !loadFailed && <p className="empty">Chargement…</p>}
      {locations && kind === 'uncovered' && (
        <p className="empty" data-testid="location-empty">
          {uncoveredText(date, today, health)}
        </p>
      )}
      {locations && kind === 'still' && (
        <p className="empty" data-testid="location-empty">
          {stillText(date, today)}
        </p>
      )}

      {locations && places && kind === 'moved' && (
        <>
          {here && <HereLine row={here} />}
          <dl className="stats">
            <div>
              <dt>Lieux</dt>
              <dd className="stat-value" data-testid="location-places">
                {places.arrived}
              </dd>
              {places.earlier && <dd className="stat-note">{places.earlier}</dd>}
            </div>
            <div>
              <dt>Distance</dt>
              <dd className="stat-value" data-testid="location-distance">
                {hasTrace(locations) ? distanceText(traceLengthM(locations)) : '–'}
              </dd>
              {!hasTrace(locations) && <dd className="stat-note">Aucune trace</dd>}
            </div>
          </dl>

          {drawing && (
            <MapFigure testId="location-map">
              <LocationMap drawing={drawing} label={mapLabel(locations)} />
            </MapFigure>
          )}

          <section ref={placesSection} className="music-section" aria-labelledby="titre-lieux" data-testid="visits">
            <h3 id="titre-lieux" className="subsection-title">
              Lieux
            </h3>
            {rows.length === 0 ? (
              <p className="empty">Aucun arrêt ce jour-là.</p>
            ) : (
              <>
                <ol className="rows visit-rows">
                  {shownRows.map((row) => (
                    <PlaceLine key={row.key} row={row} />
                  ))}
                </ol>
                <MoreButton total={rows.length} expanded={unfolded} onToggle={() => setUnfoldedOn(unfolded ? null : date)} section={placesSection} />
              </>
            )}
          </section>
        </>
      )}
    </section>
  )
}
