// Le sélecteur segmenté d'iOS. Changement instantané : un geste fréquent, et des données à lire.

interface Props<T extends string> {
  label: string
  options: readonly (readonly [T, string])[]
  value: T
  onChange: (value: T) => void
  className?: string
}

export function Segments<T extends string>({ label, options, value, onChange, className }: Props<T>) {
  return (
    <div className={className ? `segments ${className}` : 'segments'} role="group" aria-label={label}>
      {options.map(([id, text]) => (
        <button key={id} type="button" className="segment" aria-pressed={value === id} onClick={() => onChange(id)}>
          {text}
        </button>
      ))}
    </div>
  )
}
