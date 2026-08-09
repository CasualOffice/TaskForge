/**
 * The sign-in screen.
 *
 * # The failure this module prevents
 *
 * Undoing the server's constant-shape refusal. `crates/casual-task-api/src/auth.rs`
 * goes to considerable trouble — one failure variant, an Argon2 verification even
 * for an address that does not exist — so that login cannot be used to enumerate
 * accounts. A client that said "no account with that email" would hand back the
 * oracle the server spent 100 ms of CPU per request denying.
 *
 * So there is exactly one failure sentence here, it comes from the registry
 * (`TF-AUT-0001`), and nothing branches on which field was wrong.
 */
import { useState, type FormEvent, type ReactElement } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { login } from '../api/session'
import { ErrorNotice } from './notice'

export function SignIn(): ReactElement {
  const client = useQueryClient()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')

  const attempt = useMutation({
    mutationFn: () => login({ email, password }),
    onSuccess: async () => {
      // The session cookie is set; every cached "nobody is signed in" answer is
      // now wrong. Invalidating rather than setting the data: the server is the
      // authority on who the session belongs to, and guessing it here would put
      // a second copy of identity in the cache.
      await client.invalidateQueries()
    },
  })

  function submit(event: FormEvent): void {
    event.preventDefault()
    attempt.mutate()
  }

  return (
    <main className="signin">
      <form className="signin__card" onSubmit={submit} aria-labelledby="signin-heading">
        <h1 id="signin-heading" className="signin__title">
          TaskForge
        </h1>
        <p className="field__hint">Sign in to continue.</p>

        <div className="field">
          <label className="field__label" htmlFor="signin-email">
            Email
          </label>
          <input
            id="signin-email"
            className="input"
            type="email"
            name="email"
            autoComplete="username"
            required
            value={email}
            onChange={(event) => setEmail(event.target.value)}
          />
        </div>

        <div className="field">
          <label className="field__label" htmlFor="signin-password">
            Password
          </label>
          <input
            id="signin-password"
            className="input"
            type="password"
            name="password"
            autoComplete="current-password"
            required
            value={password}
            onChange={(event) => setPassword(event.target.value)}
          />
        </div>

        {attempt.isError ? <ErrorNotice error={attempt.error} /> : null}

        <button className="button button--primary signin__submit" type="submit" disabled={attempt.isPending}>
          {attempt.isPending ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </main>
  )
}
