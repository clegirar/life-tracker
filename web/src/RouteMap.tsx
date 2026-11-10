// Les cartes du jour, sur un fond OpenStreetMap : le parcours d'une séance, façon Garmin Connect (le
// tracé en couleurs d'allure, un repère à chaque kilomètre), et les déplacements de l'iPhone (les morceaux de trace, les points
// isolés, les lieux numérotés, et les séances de la montre dans leur couleur). Chargé à part (React.lazy), avec Leaflet et sa feuille de style.
// Au toucher, la carte ne capte pas le défilement : on fait défiler la page, pas la carte, sauf une fois
// zoomée au-delà de sa vue d'origine, où un doigt la déplace. La molette ne zoome jamais : nos boutons.

import { useEffect, useRef, useState } from 'react'
import L from 'leaflet'
import 'leaflet/dist/leaflet.css'
import { fetchTrack } from './api'
import { drawingPoints, type LatLng, type MapDrawing } from './location'
import { kilometreMarkers } from './kilometres'
import { paceDrawing, type PaceDrawing } from './pace'

const TILES = 'https://tile.openstreetmap.org/{z}/{x}/{y}.png'

const color = (el: Element, name: string) => getComputedStyle(el).getPropertyValue(name).trim()

type MapState = 'loading' | 'ready' | 'failed'

/** Ce que les boutons de zoom ont besoin de savoir de la vue. */
interface ViewState {
  canZoomIn: boolean
  canZoomOut: boolean
  /** La vue n'est plus celle d'origine : « Recentrer » apparaît. */
  moved: boolean
}

interface MapControls {
  zoomIn: () => void
  zoomOut: () => void
  recenter: () => void
}

const isTouch = () => window.matchMedia('(pointer: coarse)').matches

/** Une carte Leaflet sans commandes, ses tuiles, et l'appel `onReady` à la première tuile reçue. */
function createMap(el: HTMLElement, onReady: () => void): L.Map {
  const touch = isTouch()
  const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const map = L.map(el, {
    attributionControl: false,
    zoomControl: false,
    dragging: !touch,
    touchZoom: false,
    scrollWheelZoom: false,
    doubleClickZoom: !touch,
    boxZoom: false,
    keyboard: false,
    // Au-delà, la séance n'est plus qu'un point : rien à voir.
    minZoom: 3,
    // Mouvement réduit : le zoom saute d'un cran à l'autre, sans glisser.
    zoomAnimation: !still,
    markerZoomAnimation: !still,
    fadeAnimation: !still,
  })
  const tiles = L.tileLayer(TILES, {
    maxZoom: 19,
    // La politique d'OpenStreetMap demande un Referer : l'origine seule, jamais l'adresse de la page.
    referrerPolicy: 'strict-origin',
  })
  tiles.once('load', onReady)
  tiles.addTo(map)
  return map
}

/**
 * Cadre la carte par `fit`, retient cette vue d'origine et suit ses changements : les limites du zoom,
 * la vue déplacée ou non, et, au toucher, le glisser à un doigt, permis seulement plus près que la vue
 * d'origine (sinon le doigt fait défiler la page). Appelé avant le premier cadrage.
 */
function trackView(map: L.Map, fit: () => void, onView: (view: ViewState) => void): MapControls {
  const touch = isTouch()
  let home: { zoom: number; center: L.LatLng } | null = null
  let fitting = true
  // `moveend` suit chaque changement de vue, zoom compris, une fois l'animation finie.
  map.on('moveend', () => {
    const zoom = map.getZoom()
    if (fitting || !home) {
      home = { zoom, center: map.getCenter() }
      fitting = false
    }
    const shift = map.project(map.getCenter(), zoom).distanceTo(map.project(home.center, zoom))
    if (touch) {
      if (zoom > home.zoom) map.dragging.enable()
      else map.dragging.disable()
    }
    onView({ canZoomIn: zoom < map.getMaxZoom(), canZoomOut: zoom > map.getMinZoom(), moved: zoom !== home.zoom || shift > 2 })
  })
  fit()
  return {
    zoomIn: () => map.zoomIn(),
    zoomOut: () => map.zoomOut(),
    recenter: () => {
      fitting = true
      fit()
    },
  }
}

const NO_VIEW: ViewState = { canZoomIn: false, canZoomOut: false, moved: false }

/** L'état des boutons de zoom d'une carte, et ce qu'il faut à `trackView` pour le tenir. */
function useMapView() {
  const [view, setView] = useState<ViewState>(NO_VIEW)
  const [controls, setControls] = useState<MapControls | null>(null)
  return { view, setView, controls, setControls }
}

const ZOOM_IN = 'M12 5v14M5 12h14'
const ZOOM_OUT = 'M5 12h14'

/**
 * Zoomer, dézoomer et, une fois la vue changée, recentrer : de vrais boutons posés sur la carte, en bas
 * à droite, et non le contrôle de Leaflet. Grisés aux limites du zoom.
 */
function ZoomButtons({ view, controls }: { view: ViewState; controls: MapControls }) {
  return (
    <div className="map-controls">
      {view.moved && (
        <button type="button" className="map-recenter" onClick={controls.recenter}>
          Recentrer
        </button>
      )}
      <div className="map-zoom" role="group" aria-label="Zoom de la carte">
        {(
          [
            ['Zoomer', ZOOM_IN, view.canZoomIn, controls.zoomIn],
            ['Dézoomer', ZOOM_OUT, view.canZoomOut, controls.zoomOut],
          ] as const
        ).map(([name, d, enabled, onClick]) => (
          <button key={name} type="button" className="map-zoom-button" aria-label={name} disabled={!enabled} onClick={onClick}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" aria-hidden="true">
              <path d={d} />
            </svg>
          </button>
        ))}
      </div>
    </div>
  )
}

function MapFrame({
  host,
  state,
  label,
  failure,
  view,
  controls,
}: {
  host: React.RefObject<HTMLDivElement | null>
  state: MapState
  label: string
  failure: string
  view: ViewState
  controls: MapControls | null
}) {
  return (
    <>
      <div ref={host} className="route-map" data-state={state} role="img" aria-label={label} />
      {state === 'ready' && controls && <ZoomButtons view={view} controls={controls} />}
      {state === 'failed' && <p className="route-failed">{failure}</p>}
    </>
  )
}

/**
 * Un repère kilométrique, centré sur son point : un disque sombre cerclé de blanc et son numéro, lisible
 * sur toutes les couleurs d'allure. Sans taille : le disque s'élargit par CSS pour « 10 » ou « 100 ».
 */
const kilometreIcon = (km: number) =>
  L.divIcon({ className: 'route-km', html: `<span role="img" aria-label="Kilomètre ${km}">${km}</span>`, iconSize: [0, 0] })

/** La trace d'une séance, ses couleurs d'allure (ou d'une couleur si l'allure ne se lit pas), et ses repères kilométriques. */
function drawTrack(map: L.Map, el: HTMLElement, points: [number, number, number][], pace: PaceDrawing | null): L.LatLngBounds {
  const all = points.map(([lat, lon]) => [lat, lon] as L.LatLngTuple)
  let line: L.Polyline
  if (pace) {
    // Un liseré sous toute la trace, puis un morceau par suite de tronçons de même allure.
    line = L.polyline(all, { color: color(el, '--pace-casing'), weight: 7, lineJoin: 'round', lineCap: 'round', interactive: false }).addTo(map)
    for (const run of pace.runs) {
      L.polyline(run.points as L.LatLngTuple[], {
        color: color(el, `--pace-${run.level}`),
        weight: 4,
        opacity: 1,
        lineJoin: 'round',
        lineCap: 'round',
        interactive: false,
      }).addTo(map)
    }
  } else {
    line = L.polyline(all, { color: color(el, '--workout'), weight: 4, opacity: 0.95, lineJoin: 'round', interactive: false }).addTo(map)
  }
  for (const { km, at } of kilometreMarkers(points)) {
    L.marker(at as L.LatLngTuple, { icon: kilometreIcon(km), interactive: false, keyboard: false }).addTo(map)
  }
  return line.getBounds()
}

/** Les bornes de l'échelle d'allure d'une séance, en m/s, pour la légende sous la carte. */
export interface PaceScale {
  slow: number
  fast: number
}

export default function RouteMap({ activityId, label, onPace }: { activityId: number; label: string; onPace?: (scale: PaceScale | null) => void }) {
  const host = useRef<HTMLDivElement>(null)
  const [state, setState] = useState<MapState>('loading')
  const { view, setView, controls, setControls } = useMapView()

  useEffect(() => {
    const el = host.current
    if (!el) return
    let map: L.Map | null = null
    let cancelled = false
    fetchTrack(activityId).then(
      (track) => {
        if (cancelled || track.points.length < 2) return
        const fitted = createMap(el, () => setState('ready'))
        map = fitted
        const pace = paceDrawing(track.points)
        const bounds = drawTrack(fitted, el, track.points, pace)
        setControls(trackView(fitted, () => fitted.fitBounds(bounds, { padding: [20, 20] }), setView))
        onPace?.(pace && { slow: pace.slow, fast: pace.fast })
      },
      () => {
        if (!cancelled) setState('failed')
      },
    )
    return () => {
      cancelled = true
      map?.remove()
    }
  }, [activityId, onPace, setView, setControls])

  return <MapFrame host={host} state={state} label={label} failure="Parcours indisponible." view={view} controls={controls} />
}

/**
 * Un lieu de la journée : une pastille numérotée, le numéro de sa ligne dans la liste (« 1 · 4 » pour
 * deux lieux au même endroit). Sans taille : la pastille, centrée sur le point par CSS, s'élargit. Le
 * lieu où je suis encore porte un halo fixe, le même que sa pastille en haut de la carte.
 */
const pinIcon = (label: string, current: boolean) =>
  L.divIcon({ className: current ? 'visit-pin visit-pin-current' : 'visit-pin', html: `<span>${label}</span>`, iconSize: [0, 0] })

/**
 * Les déplacements d'un jour, déjà lus : la page les relit toutes les 5 min, mais un dessin identique
 * garde le même objet (`useLocations`) et la carte ne se refait pas.
 */
export function LocationMap({ drawing, label }: { drawing: MapDrawing; label: string }) {
  const host = useRef<HTMLDivElement>(null)
  const [state, setState] = useState<MapState>('loading')
  const { view, setView, controls, setControls } = useMapView()

  useEffect(() => {
    const el = host.current
    if (!el) return
    const map = createMap(el, () => setState('ready'))
    const tone = color(el, '--location')
    const style = { color: tone, weight: 4, opacity: 0.9, lineJoin: 'round', lineCap: 'round', interactive: false } as const
    for (const line of drawing.lines) L.polyline(line as L.LatLngTuple[], style).addTo(map)
    // Une séance de la montre : la même ligne, dans la couleur des séances.
    const workout = { ...style, color: color(el, '--workout') }
    for (const line of drawing.workouts) L.polyline(line as L.LatLngTuple[], workout).addTo(map)
    // Un point isolé : un petit rond, pour ne pas le confondre avec un lieu.
    for (const dot of drawing.dots) {
      L.circleMarker(dot as L.LatLngTuple, { radius: 4, color: '#ffffff', weight: 1.5, fillColor: tone, fillOpacity: 1, interactive: false }).addTo(map)
    }
    for (const pin of drawing.pins) {
      // Le lieu en cours passe devant les autres épingles.
      L.marker(pin.at as L.LatLngTuple, { icon: pinIcon(pin.label, pin.current), zIndexOffset: pin.current ? 1000 : 0, interactive: false, keyboard: false }).addTo(map)
    }
    // Un seul endroit (un lieu, un point) : un zoom de quartier, pas le plus fort.
    const bounds = L.latLngBounds(drawingPoints(drawing).map((point: LatLng) => point as L.LatLngTuple))
    setControls(trackView(map, () => map.fitBounds(bounds, { padding: [28, 28], maxZoom: 16 }), setView))
    return () => {
      map.remove()
    }
  }, [drawing, setView, setControls])

  return <MapFrame host={host} state={state} label={label} failure="Carte indisponible." view={view} controls={controls} />
}
