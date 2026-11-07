import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from 'react'
import type { SourcesHealth, Workout } from './api'
import { integerText, isEmpty, kmText, sleepHeartRate, sleepSpan, sleepStages, sleepText, speedText, workoutFigures, workoutType } from './activity'
import { PACE_CLASSES } from './pace'
import type { PaceScale } from './RouteMap'
import type { useActivity } from './useActivity'

// Leaflet et ses tuiles ne se chargent qu'à l'approche d'une carte : le reste de l'app n'en paie rien.
const RouteMap = lazy(() => import('./RouteMap'))

/** Les métriques de la carte, chacune un bloc qu'un filtre peut masquer. */
export const ACTIVITY_METRICS = ['steps', 'calories.burned', 'sleep', 'workout']

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useActivity>
  health: SourcesHealth | null
  /** Les métriques cochées parmi `ACTIVITY_METRICS` : la page ne monte pas la carte sans aucune. */
  shown: string[]
}

/**
 * Une carte du jour (séance, déplacements), montée quand elle approche de l'écran ; d'ici là, un cadre
 * de même taille. `children` est la carte chargée à part (React.lazy) : rien ne se charge avant.
 */
export function MapFigure({ testId, children, legend }: { testId: string; children: ReactNode; legend?: ReactNode }) {
  const frame = useRef<HTMLDivElement>(null)
  const [near, setNear] = useState(false)
  useEffect(() => {
    const el = frame.current
    if (!el || near) return
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) setNear(true)
      },
      { rootMargin: '200px' },
    )
    observer.observe(el)
    return () => observer.disconnect()
  }, [near])
  return (
    <figure className="route" data-testid={testId}>
      <div ref={frame} className="route-frame">
        {near && <Suspense fallback={null}>{children}</Suspense>}
      </div>
      <figcaption className="route-credit">
        {legend}
        <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noreferrer">
          © les contributeurs d’OpenStreetMap
        </a>
      </figcaption>
    </figure>
  )
}

/** Sous la carte : « Lent 7:12 /km ▬▬▬ Rapide 5:01 /km », les bornes de l'échelle de couleurs. */
function PaceLegend({ scale, type }: { scale: PaceScale; type: string }) {
  return (
    <span className="pace-legend" data-testid="pace-legend">
      <span className="visually-hidden">Couleur de la trace selon l’allure : </span>
      <span>
        Lent <strong>{speedText(scale.slow, type)}</strong>
      </span>
      <span className="pace-scale" aria-hidden="true">
        {Array.from({ length: PACE_CLASSES }, (_, i) => (
          <span key={i} />
        ))}
      </span>
      <span>
        Rapide <strong>{speedText(scale.fast, type)}</strong>
      </span>
    </span>
  )
}

function WorkoutRow({ workout }: { workout: Workout }) {
  const { details } = workout
  const type = workoutType(details.type)
  const figures = workoutFigures(details)
  const [pace, setPace] = useState<PaceScale | null>(null)
  return (
    <li className="workout" data-testid="workout">
      <div className="workout-head">
        <p className="row-name">{type}</p>
        <p className="row-detail">{[workout.time, details.location_name].filter(Boolean).join(' · ')}</p>
      </div>
      {figures.length > 0 && (
        <dl className="workout-figures">
          {figures.map((figure) => (
            <div key={figure.label}>
              <dt>{figure.label}</dt>
              <dd>{figure.value}</dd>
            </div>
          ))}
        </dl>
      )}
      {workout.has_track && (
        <MapFigure testId="route-map" legend={pace && <PaceLegend scale={pace} type={details.type} />}>
          <RouteMap activityId={details.activity_id} label={`Parcours : ${type.toLowerCase()} de ${workout.time}`} onPace={setPace} />
        </MapFigure>
      )}
    </li>
  )
}

const noData = <dd className="stat-note">Pas de données</dd>

export function ActivityCard({ date, today, state, health, shown }: Props) {
  const { activity, loadFailed } = state
  const silent = health?.sources.find((source) => /^garmin\b/.test(source.name) && source.silent)
  const stages = activity?.sleep ? sleepStages(activity.sleep.details) : []
  const stageTotal = stages.reduce((sum, stage) => sum + stage.minutes, 0)
  const span = activity?.sleep ? sleepSpan(activity.sleep.details) : null
  const heartRate = activity?.sleep ? sleepHeartRate(activity.sleep.details) : null

  return (
    <section className="card card-activity" aria-labelledby="titre-activite" data-testid="activity">
      <h2 id="titre-activite" className="card-title card-title-activity">
        Activité
      </h2>

      {loadFailed && (
        <div className="inline-failure" role="status">
          <span>Impossible d'actualiser l'activité.</span>
          <button type="button" className="notice-action" onClick={() => void state.reload()}>
            Réessayer
          </button>
        </div>
      )}
      {silent && (
        <p className="inline-note" role="status" data-testid="garmin-alert">
          <strong>Garmin ne répond plus.</strong> Des données de ce jour peuvent manquer.
        </p>
      )}

      {!activity && !loadFailed && <p className="empty">Chargement…</p>}
      {activity && isEmpty(activity) && (
        <p className="empty">{date === today ? "Pas encore de données Garmin aujourd'hui." : 'Pas de données Garmin ce jour-là.'}</p>
      )}

      {activity && !isEmpty(activity) && (
        <>
          {shown.some((metric) => metric !== 'workout') && (
            <dl className="stats">
              {shown.includes('steps') && (
                <div>
                  <dt>Pas</dt>
                  <dd className="stat-value" data-testid="steps">
                    {activity.steps?.value != null ? integerText(activity.steps.value) : '–'}
                  </dd>
                  {activity.steps === null
                    ? noData
                    : activity.steps.details.distance_m !== null && <dd className="stat-note">{kmText(activity.steps.details.distance_m)}</dd>}
                </div>
              )}
              {shown.includes('calories.burned') && (
                <div>
                  <dt>Calories</dt>
                  <dd className="stat-value" data-testid="calories">
                    {activity.calories?.value != null ? integerText(activity.calories.value) : '–'}
                    {activity.calories?.value != null && <span className="stat-unit"> kcal</span>}
                  </dd>
                  {activity.calories === null ? (
                    noData
                  ) : (
                    <dd className="stat-note">
                      {[
                        activity.calories.details.active !== null && `${integerText(activity.calories.details.active)} actives`,
                        activity.calories.details.resting !== null && `${integerText(activity.calories.details.resting)} au repos`,
                      ]
                        .filter(Boolean)
                        .join(' · ')}
                    </dd>
                  )}
                </div>
              )}
              {shown.includes('sleep') && (
                <div className="stat-wide">
                  <dt>Sommeil</dt>
                  <dd className="stat-value" data-testid="sleep">
                    {activity.sleep?.value != null ? sleepText(activity.sleep.value) : '–'}
                  </dd>
                  {activity.sleep === null ? noData : span && <dd className="stat-note">{span}</dd>}
                  {heartRate && (
                    <dd className="stat-note" data-testid="sleep-heart-rate">
                      {heartRate}
                    </dd>
                  )}
                  {stageTotal > 0 && (
                    <dd className="sleep-stages">
                      <span className="sleep-bar" aria-hidden="true">
                        {stages.map((stage) => (
                          <span key={stage.key} className={`sleep-part sleep-${stage.key}`} style={{ flexGrow: stage.minutes }} />
                        ))}
                      </span>
                      <ul className="sleep-legend">
                        {stages.map((stage) => (
                          <li key={stage.key}>
                            <span className={`sleep-swatch sleep-${stage.key}`} aria-hidden="true" />
                            {stage.label} <span className="sleep-minutes">{sleepText(stage.minutes)}</span>
                          </li>
                        ))}
                      </ul>
                    </dd>
                  )}
                </div>
              )}
            </dl>
          )}

          {shown.includes('workout') && (
            <section className="music-section" aria-labelledby="titre-seances" data-testid="workouts">
              <h3 id="titre-seances" className="subsection-title">
                Séances
                {activity.workouts.length > 0 && <span className="subsection-count subsection-count-activity">{activity.workouts.length}</span>}
              </h3>
              {activity.workouts.length === 0 ? (
                <p className="empty">Aucune séance ce jour-là.</p>
              ) : (
                <ul className="rows">
                  {activity.workouts.map((workout) => (
                    <WorkoutRow key={workout.details.activity_id} workout={workout} />
                  ))}
                </ul>
              )}
            </section>
          )}
        </>
      )}
    </section>
  )
}
