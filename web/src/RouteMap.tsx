// Les cartes du jour, sur un fond OpenStreetMap : le parcours d'une séance, façon Garmin Connect (le
// tracé, le départ et l'arrivée), et les déplacements de l'iPhone (les morceaux de trace, les points
// isolés et les lieux numérotés). Chargé à part (React.lazy), avec Leaflet et sa feuille de style.
// Au toucher, la carte ne capte pas le défilement : on fait défiler la page, pas la carte.

import { useEffect, useRef, useState } from 'react'
import L from 'leaflet'
import 'leaflet/dist/leaflet.css'
import { fetchTrack } from './api'
import { drawingPoints, type LatLng, type MapDrawing } from './location'

const TILES = 'https://tile.openstreetmap.org/{z}/{x}/{y}.png'

const color = (el: Element, name: string) => getComputedStyle(el).getPropertyValue(name).trim()

type MapState = 'loading' | 'ready' | 'failed'

/** Une carte Leaflet sans commandes, ses tuiles, et l'appel `onReady` à la première tuile reçue. */
function createMap(el: HTMLElement, onReady: () => void): L.Map {
  const touch = window.matchMedia('(pointer: coarse)').matches
  const map = L.map(el, {
    attributionControl: false,
    zoomControl: false,
    dragging: !touch,
    touchZoom: false,
    scrollWheelZoom: false,
    doubleClickZoom: !touch,
    boxZoom: false,
    keyboard: false,
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

function MapFrame({ host, state, label, failure }: { host: React.RefObject<HTMLDivElement | null>; state: MapState; label: string; failure: string }) {
  return (
    <>
      <div ref={host} className="route-map" data-state={state} role="img" aria-label={label} />
      {state === 'failed' && <p className="route-failed">{failure}</p>}
    </>
  )
}

export default function RouteMap({ activityId, label }: { activityId: number; label: string }) {
  const host = useRef<HTMLDivElement>(null)
  const [state, setState] = useState<MapState>('loading')

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
        map.fitBounds(line.getBounds(), { padding: [20, 20] })
      },
      () => {
        if (!cancelled) setState('failed')
      },
    )
    return () => {
      cancelled = true
      map?.remove()
    }
  }, [activityId])

  return <MapFrame host={host} state={state} label={label} failure="Parcours indisponible." />
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
    const points = drawingPoints(drawing)
    map.fitBounds(L.latLngBounds(points.map((point: LatLng) => point as L.LatLngTuple)), { padding: [28, 28], maxZoom: 16 })
    return () => {
      map.remove()
    }
  }, [drawing])

  return <MapFrame host={host} state={state} label={label} failure="Carte indisponible." />
}
