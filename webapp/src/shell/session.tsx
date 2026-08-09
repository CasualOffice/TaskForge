/**
 * Who is signed in, and which workspace they are looking at.
 *
 * # The failure this module prevents
 *
 * A tenant-scoped request sent with no workspace, or with the previous one.
 * Every call in `api/` takes a `workspaceId` as its first argument precisely so
 * it cannot be forgotten — and that only helps if there is exactly one place the
 * value comes from. This is that place.
 *
 * # Why the workspace is context and not a route parameter
 *
 * `docs/05` lets the workspace come from the path *or* the `X-Workspace-Id`
 * header, and the header is what a session-authenticated browser uses: the
 * routes this app has (`/board`, `/list`, `/my-work`) are not workspace-scoped
 * paths, so a route parameter would be a second copy of a value the header
 * already carries. The chosen workspace is persisted so a reload does not drop
 * the user into a workspace picker they already answered.
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readSession, type SessionView } from '../api/session'
import { listWorkspaces, type Workspace } from '../api/workspaces'
import { applyWorkspacePrimary } from './appearance'

/** Where the chosen workspace survives a reload. */
const STORED_WORKSPACE = 'taskforge.workspace'

interface SessionState {
  /** `null` once the session query has answered and nobody is signed in. */
  readonly actor: SessionView | null
  readonly loading: boolean
  /** The credential was not rejected; its status could not be checked. */
  readonly sessionUnavailable: boolean
  /** The actor is signed in; their workspace directory could not be loaded. */
  readonly workspacesUnavailable: boolean
  readonly workspaces: readonly Workspace[]
  readonly workspace: Workspace | undefined
  readonly chooseWorkspace: (id: string) => void
  readonly retrySession: () => void
  readonly retryWorkspaces: () => void
  /** Drops every cached tenant row. Called after logout, never speculatively. */
  readonly forget: () => void
}

const SessionContext = createContext<SessionState | undefined>(undefined)

export function SessionProvider({ children }: { children: ReactNode }): ReactElement {
  const client = useQueryClient()
  const [chosen, setChosen] = useState<string | null>(() =>
    localStorage.getItem(STORED_WORKSPACE),
  )

  const session = useQuery({
    queryKey: keys.session(),
    queryFn: ({ signal }) => readSession(signal),
    // A session is not stale data to be refetched aggressively; it changes
    // exactly twice per visit. Refetching it on every window focus would put an
    // uncacheable database read behind every tab switch (`docs/40`: sessions are
    // never cached server-side either).
    staleTime: 60_000,
    retry: false,
  })

  const signedIn = session.data != null

  const workspaces = useQuery({
    queryKey: keys.workspaces(),
    queryFn: ({ signal }) => listWorkspaces(signal),
    enabled: signedIn,
    staleTime: 60_000,
  })
  const refetchSession = session.refetch
  const refetchWorkspaces = workspaces.refetch
  const retrySession = useCallback(() => void refetchSession(), [refetchSession])
  const retryWorkspaces = useCallback(() => void refetchWorkspaces(), [refetchWorkspaces])

  const available = useMemo(() => workspaces.data?.data ?? [], [workspaces.data])

  // The stored choice, if it is still one of theirs; otherwise the first. A
  // workspace the user was removed from is in localStorage until they clear it,
  // and honouring it would send `X-Workspace-Id` for a tenant every request 404s
  // on — which reads as "the app is broken", not "you were removed".
  const workspace = useMemo(
    () => available.find((candidate) => candidate.id === chosen) ?? available[0],
    [available, chosen],
  )

  // A stale stored tenant is a preference, never authority. Once the current
  // list answers, replace an unavailable choice and share the correction with
  // the next tab/reload.
  useEffect(() => {
    if (!workspaces.isSuccess) return
    if (workspace === undefined) {
      localStorage.removeItem(STORED_WORKSPACE)
      if (chosen !== null) setChosen(null)
      return
    }
    if (chosen !== workspace.id) {
      setChosen(workspace.id)
      localStorage.setItem(STORED_WORKSPACE, workspace.id)
    }
  }, [chosen, workspace, workspaces.isSuccess])

  useEffect(() => {
    applyWorkspacePrimary(workspace?.appearance?.primary_color)
  }, [workspace?.appearance?.primary_color])

  // A workspace switch in another tab updates presentation context without
  // sharing any credential or tenant response body through browser storage.
  useEffect(() => {
    function onStorage(event: StorageEvent): void {
      if (event.key === STORED_WORKSPACE) setChosen(event.newValue)
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [])

  const chooseWorkspace = useCallback(
    (id: string) => {
      setChosen(id)
      localStorage.setItem(STORED_WORKSPACE, id)
      // Tenant rows are keyed under `['ws', id, …]`, so nothing from the old
      // workspace can be read after this — but the memory would otherwise be
      // held for the life of the tab.
      void client.invalidateQueries({ queryKey: ['ws'] })
    },
    [client],
  )

  const forget = useCallback(() => {
    localStorage.removeItem(STORED_WORKSPACE)
    setChosen(null)
    client.clear()
  }, [client])

  const value = useMemo<SessionState>(
    () => ({
      actor: session.data ?? null,
      loading: session.isPending || (signedIn && workspaces.isPending),
      sessionUnavailable: session.isError,
      workspacesUnavailable: signedIn && workspaces.isError,
      workspaces: available,
      workspace,
      chooseWorkspace,
      retrySession,
      retryWorkspaces,
      forget,
    }),
    [
      session.data,
      session.isPending,
      session.isError,
      signedIn,
      workspaces.isPending,
      workspaces.isError,
      available,
      workspace,
      chooseWorkspace,
      retrySession,
      retryWorkspaces,
      forget,
    ],
  )

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>
}

/** The session. Throws outside the provider, which is a wiring bug, not a state. */
export function useSession(): SessionState {
  const value = useContext(SessionContext)
  if (value === undefined) throw new Error('useSession outside SessionProvider')
  return value
}

/**
 * The current workspace id, for a component that cannot render without one.
 *
 * Returns `''` rather than throwing while the list is loading: the shell does
 * not mount tenant views until a workspace exists, so an empty string here means
 * a component rendered outside that guard — visible immediately as a failing
 * request rather than as a blank screen.
 */
export function useWorkspaceId(): string {
  return useSession().workspace?.id ?? ''
}
