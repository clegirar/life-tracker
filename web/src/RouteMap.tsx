// Les cartes du jour, sur un fond OpenStreetMap : le parcours d'une séance, façon Garmin Connect (le
// tracé, le départ et l'arrivée), et les déplacements de l'iPhone (les morceaux de trace, les points
// isolés et les lieux numérotés). Chargé à part (React.lazy), avec Leaflet et sa feuille de style.
// Au toucher, la carte ne capte pas le défilement : on fait défiler la page, pas la carte, sauf une fois
// zoomée au-delà de sa vue d'origine, où un doigt la déplace. La molette ne zoome jamais : nos boutons.

import { useEffect, useRef, useState } from 'react'
import L from 'leaflet'
import 'leaflet/dist/leaflet.css'
import { fetchTrack } from './api'
import { drawingPoints, type LatLng, type MapDrawing } from './location'

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

export default function RouteMap({ activityId, label }: { activityId: number; label: string }) {
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
        map = createMap(el, () => setState('ready'))
        const line = L.polyline(
          track.points.map(([lat, lon]) => [lat, lon] as L.LatLngTuple),
          { color: color(el, '--workout'), weight: 4, opacity: 0.95, lineJoin: 'round', interactive: false },
        ).addTo(map)
        const marker = (at: [number, number, number], fill: string) =>
          L.circleMarker([at[0], at[1]], { radius: 6, color: '#ffffff', weight: 2, fillColor: fill, fillOpacity: 1, interactive: false }).addTo(map!)
        marker(track.points[0], color(el, '--steps'))
        marker(track.points[track.points.length - 1], color(el, '--music'))
        const bounds = line.getBounds()
        const fitted = map
        setControls(trackView(fitted, () => fitted.fitBounds(bounds, { padding: [20, 20] }), setView))
      },
      () => {
        if (!cancelled) setState('failed')
      },
    )
    return () => {
      cancelled = true
      map?.remove()
    }
  }, [activityId, setView, setControls])

  return <MapFrame host={host} state={state} label={label} failure="Parcours indisponible." view={view} controls={controls} />
}

/**
 * Un lieu de la journée : une pastille numérotée, le numéro de sa ligne dans la liste (« 1 · 4 » pour
 * deux lieux au même endroit). Sans taille : la pastille, centrée sur le point par CSS, s'élargit.
 */
const pinIcon = (label: string) => L.divIcon({ className: 'visit-pin', html: `<span>${label}</span>`, iconSize: [0, 0] })

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
    // Un point isolé : un petit rond, pour ne pas le confondre avec un lieu.
    for (const dot of drawing.dots) {
      L.circleMarker(dot as L.LatLngTuple, { radius: 4, color: '#ffffff', weight: 1.5, fillColor: tone, fillOpacity: 1, interactive: false }).addTo(map)
    }
    for (const pin of drawing.pins) L.marker(pin.at as L.LatLngTuple, { icon: pinIcon(pin.label), interactive: false, keyboard: false }).addTo(map)
    // Un seul endroit (un lieu, un point) : un zoom de quartier, pas le plus fort.
    const bounds = L.latLngBounds(drawingPoints(drawing).map((point: LatLng) => point as L.LatLngTuple))
    setControls(trackView(map, () => map.fitBounds(bounds, { padding: [28, 28], maxZoom: 16 }), setView))
    return () => {
      map.remove()
    }
  }, [drawing, setView, setControls])

  return <MapFrame host={host} state={state} label={label} failure="Carte indisponible." view={view} controls={controls} />
}
