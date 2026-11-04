// La carte d'une séance, façon Garmin Connect : le tracé sur un fond OpenStreetMap, le départ et
// l'arrivée. Chargé à part (React.lazy), avec Leaflet et sa feuille de style.
// Au toucher, la carte ne capte pas le défilement : on fait défiler la page, pas la carte.

import { useEffect, useRef, useState } from 'react'
import L from 'leaflet'
import 'leaflet/dist/leaflet.css'
import { fetchTrack } from './api'

const TILES = 'https://tile.openstreetmap.org/{z}/{x}/{y}.png'

const color = (el: Element, name: string) => getComputedStyle(el).getPropertyValue(name).trim()

export default function RouteMap({ activityId, label }: { activityId: number; label: string }) {
  const host = useRef<HTMLDivElement>(null)
  const [state, setState] = useState<'loading' | 'ready' | 'failed'>('loading')

  useEffect(() => {
    const el = host.current
    if (!el) return
    let map: L.Map | null = null
    let cancelled = false
    fetchTrack(activityId).then(
      (track) => {
        if (cancelled || track.points.length < 2) return
        const touch = window.matchMedia('(pointer: coarse)').matches
        map = L.map(el, {
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
        tiles.once('load', () => setState('ready'))
        tiles.addTo(map)
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

  return (
    <>
      <div ref={host} className="route-map" data-state={state} role="img" aria-label={label} />
      {state === 'failed' && <p className="route-failed">Parcours indisponible.</p>}
    </>
  )
}
