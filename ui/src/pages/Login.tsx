import { useState, type FormEvent } from 'react'
import { api, ApiError, errorMessage } from '../api.ts'
import logo from '../assets/featherpanel-logo.png'

interface Props {
  onLogin: () => Promise<void>
  passwordConfigured: boolean
}

export default function Login({ onLogin, passwordConfigured }: Props) {
  const [password, setPassword] = useState('')
  const [show, setShow] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await api.post('/auth/login', { password })
      await onLogin()
    } catch (err) {
      setError(err instanceof ApiError && err.status === 401 ? 'Invalid credentials' : errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="min-vh-100 d-flex align-items-center justify-content-center bg-body-tertiary p-3">
      <form className="card shadow-sm" style={{ width: '100%', maxWidth: 380 }} onSubmit={submit}>
        <div className="card-body p-4">
          <div className="text-center mb-4">
            <img src={logo} alt="FeatherPanel" className="ft-logo mb-2" style={{ width: 56, height: 56 }} />
            <h4 className="mb-0">FeatherTail</h4>
            <small className="text-body-secondary">Sign in</small>
          </div>
          <label className="form-label">{passwordConfigured ? 'Password or token' : 'Token'}</label>
          <div className="input-group mb-3">
            <input
              type={show ? 'text' : 'password'}
              className="form-control"
              value={password}
              autoFocus
              autoComplete="current-password"
              onChange={(e) => setPassword(e.target.value)}
            />
            <button type="button" className="btn btn-outline-secondary" onClick={() => setShow(!show)} tabIndex={-1}>
              <i className={`bi bi-eye${show ? '-slash' : ''}`} />
            </button>
          </div>
          {error && <div className="alert alert-danger py-2">{error}</div>}
          <button className="btn btn-primary w-100" disabled={busy || !password}>
            {busy ? <span className="spinner-border spinner-border-sm" /> : 'Sign in'}
          </button>
        </div>
      </form>
    </div>
  )
}
