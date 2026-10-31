import { useState, type FormEvent } from 'react'
import { login } from './api'

export function Login({ expired, onLoggedIn }: { expired: boolean; onLoggedIn: () => void }) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(event: FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError(null)
    try {
      if (await login(username, password)) {
        onLoggedIn()
        return
      }
      setError('Identifiant ou mot de passe incorrect.')
    } catch {
      setError('Connexion impossible. Réessayez.')
    }
    setBusy(false)
  }

  return (
    <main className="page login">
      <h1 className="title">Connexion</h1>
      {expired && !error && (
        <p className="notice" role="status">
          Session expirée. Reconnectez-vous.
        </p>
      )}
      <form className="card login-form" onSubmit={submit}>
        <label className="field">
          Identifiant
          <input
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            name="username"
            autoComplete="username"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            enterKeyHint="next"
            required
          />
        </label>
        <label className="field">
          Mot de passe
          <input
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            name="password"
            type="password"
            autoComplete="current-password"
            enterKeyHint="go"
            required
          />
        </label>
        {error && (
          <p className="field-error" role="alert">
            {error}
          </p>
        )}
        <button className="primary" type="submit" disabled={busy}>
          Se connecter
        </button>
      </form>
    </main>
  )
}
