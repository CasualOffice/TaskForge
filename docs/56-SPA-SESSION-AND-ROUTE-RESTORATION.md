# 56 — SPA Session and Route Restoration

## Outcome

Refreshing a valid TaskForge URL preserves the authenticated session, workspace,
route, filters, sorting, grouping, and open task. A transient network or server
failure never presents the sign-in form as if the credential disappeared.
Production serves browser routes through an explicit SPA fallback while API,
health, metrics, and missing asset paths retain their real status codes.

This note is final. Refresh-safe routing was approved on 2026-08-09.

## Research (sources + dates checked)

Checked 2026-08-09:

- [TanStack Router deployment guidance](https://tanstack.com/router/latest/docs/how-to/deploy-to-production)
  requires history-routed SPAs to rewrite application routes to `index.html`.
- [TanStack Router history guidance](https://tanstack.com/router/latest/docs/guide/history-types)
  distinguishes browser history from hash history; TaskForge keeps normal URLs.
- [MDN Fetch credentials](https://developer.mozilla.org/en-US/docs/Web/API/Fetch_API/Using_Fetch)
  documents credential inclusion for cookie-authenticated requests.
- [MDN Set-Cookie](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie)
  documents the cookie flags already fixed by [40](40-IDENTITY-AUTH-AND-SESSION.md).

## Design

### Domain / schema impact

None. The existing opaque 30-day session cookie and server-side idle expiry are
the authority. The browser stores only presentation context: selected workspace
and URL. It never stores the session or a bearer credential.

### Session state machine

The shell models these states explicitly:

| State | Evidence | UI |
| --- | --- | --- |
| `checking` | session request pending | stable branded shell skeleton |
| `authenticated` | session endpoint returned an actor | requested route |
| `anonymous` | session endpoint returned 401 | sign-in |
| `unavailable` | network, 5xx, malformed proxy response | recovery state with Retry |
| `no-workspace` | actor valid, workspace list empty | workspace blank slate / invitation guidance |
| `workspace-unavailable` | actor valid, workspace list failed | scoped recovery state; actor stays signed in |

Only a 401 from `/auth/session` produces `anonymous`. An abort, offline error,
5xx, or invalid response produces `unavailable`. A workspace-list failure never
changes the actor. Retry happens in place and the browser URL is not replaced.

The selected workspace remains a validated local preference: use it only when
it appears in the actor's current list, otherwise select the first available
workspace and replace the stored preference. Cross-tab changes use the `storage`
event; cached tenant queries remain keyed by workspace id.

### URL contract

All restorable view state remains in TanStack Router's validated path and search
parameters. The server never redirects an authenticated application route to
`/`, `/dashboard`, or `/login`. Sign-in is a rendered auth state over the
requested URL. After successful sign-in, that same URL resumes.

The router provides a typed not-found state with a CTA back to `My Work`; it does
not turn an unknown route into the task list silently.

### Production delivery

The deployable image builds `webapp/dist` in a pinned Node/pnpm stage and copies
the immutable output to `/app/webapp`. The API process serves that directory
through `tower-http`, the Tower ecosystem's static-file layer.

Routing precedence is fixed:

1. `/api/*`, `/health/*`, and `/metrics` reach their registered handlers.
2. A request for a real fingerprinted asset returns that asset with immutable
   cache headers.
3. A missing path containing a file extension returns `404`.
4. A `GET` or `HEAD` application navigation accepting HTML returns
   `/app/webapp/index.html` with `no-cache`.
5. Other methods and media types return `404` or `405`; they never receive HTML.

Fallback is attached inside `server::router` before the common observability and
security layers so it cannot bypass request ids. API errors can therefore never
be hidden behind a successful HTML response.

The runtime path is configurable with `TF_WEB_ROOT`; the production image sets
it to `/app/webapp`. When it is set, the process refuses startup if the
directory or `index.html` is absent. API-only development and tests may leave it
unset and use the Vite same-origin proxy.

### Layers & crates touched

- `webapp/src/shell/session.tsx`: explicit state machine and retry actions.
- `webapp/src/shell/AppFrame.tsx`: stable checking/recovery/no-workspace states.
- `webapp/src/router.tsx`: typed not-found behavior; URL remains authoritative.
- `crates/casual-task-api/src/server.rs`: static assets and constrained fallback.
- `crates/casual-task-api/src/config.rs`: bounded `TF_WEB_ROOT` configuration.
- `Dockerfile`: reproducible web build stage and runtime asset copy.

### API surface

No JSON endpoint changes. Browser delivery adds document responses outside
`/api/v1`; it does not alter the versioned API.

### Failure modes & limits

- Session request cannot reach the API: show Retry and preserve route; do not
  clear Query cache or workspace preference.
- Session returns 401: clear tenant cache and show sign-in at the same URL.
- Stored workspace is no longer available: remove it and choose the first
  current membership.
- Asset absent: real 404, never `index.html`.
- Web root absent at startup: fail with a path-only diagnostic; do not log HTML
  or environment secrets.
- Old `index.html` with new assets: `no-cache` on the document plus immutable
  fingerprinted assets prevents a long-lived mixed release.
- Service worker: none in this increment. It would add a second cache and an
  update protocol that has not been designed.

### Security & tenancy implications

The fallback does not participate in authorization and exposes no tenant data;
it serves the same public shell bytes to every visitor. Authenticated data still
requires the session cookie and a per-request workspace membership check. Paths
are resolved by the static-file service under one configured root; user input is
never joined to a filesystem path in application code.

## Alternatives considered

- **Hash routing.** Rejected because it weakens readable, shareable task URLs and
  moves deployment correctness into every client link.
- **A separate reverse-proxy container.** Rejected for the single-node profile:
  it adds an artifact handoff and a process for a bounded static-delivery need.
  A scaled deployment may still put a CDN or proxy in front of the same paths.
- **Embedding assets in the Rust binary.** Rejected because every frontend edit
  invalidates a Rust link layer and increases binary memory. Files beside the
  binary retain independent caching.
- **Redirecting failures to `/login`.** Rejected because it destroys route
  context and misrepresents infrastructure failure as an authentication state.

## Acceptance gates

1. A browser signs in, opens a filtered board and task, refreshes, and returns to
   the exact URL and visible context without another login.
2. Deep-link `GET` and `HEAD` requests return `index.html`; missing `.js`, `.css`,
   image, `/api/*`, health, and metrics paths never use the fallback.
3. Offline and 5xx session tests render recovery, preserve local workspace and
   URL, and recover through Retry. A 401 alone renders sign-in and clears tenant
   cache.
4. Docker image verification starts the published image and refreshes at least
   `/board`, `/my-work`, and `/tasks/{id}`.
5. Cache headers are asserted for `index.html` and fingerprinted assets.
6. The client boot test mounts every auth/session state and every application
   route.

## ADRs triggered

- **ADR-034** — the API image serves compiled SPA files with a constrained
  history fallback through `tower-http`; new dependency and deployment surface.

## Tracker IDs

- **D-067** — SPA delivery, session state, and route restoration.
- **C-027** — production asset delivery and refresh-safe shell.
