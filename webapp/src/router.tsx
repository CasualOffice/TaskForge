/**
 * The route tree, and the one search parameter every route shares.
 *
 * # Why the open task is a search parameter and not a path
 *
 * docs/42 §Rendering strategy: "Detail opens in a drawer over the board,
 * preserving scroll position and context, with a full-page route retained for
 * deep links and new tabs." A path segment cannot express that — navigating to
 * `/tasks/{id}` unmounts the board underneath, and coming back re-fetches and
 * re-scrolls it. `?task={id}` keeps the view mounted *and* stays a real URL that
 * can be copied, bookmarked, and opened in a new tab.
 *
 * # Why only Reports is lazy
 *
 * docs/42 §What is in the shell names the board, the list, the drawer and the
 * palette as shell, and reports as lazy-always. Splitting a route the user
 * reaches in the first five seconds trades 3 KiB of budget for a spinner on the
 * critical path, which is the wrong direction for a "first usable shell < 2.5 s"
 * target.
 */
import { lazy, Suspense, type ReactElement } from 'react'
import { createRootRoute, createRoute, createRouter, type AnyRoute } from '@tanstack/react-router'

import { SettingsLayout } from './settings/SettingsLayout'
import { AppFrame } from './shell/AppFrame'
import { TaskDetail } from './task/TaskDetail'
import { BoardView } from './views/BoardView'
import { HomeView } from './views/HomeView'
import { MyWorkView } from './views/MyWorkView'
import { TaskListView } from './views/TaskListView'

const ReportsView = lazy(() => import('./views/ReportsView'))
/** The second clock's surface. Lazy for the same reason Reports is. */
const EnvironmentView = lazy(() => import('./views/EnvironmentView'))

/**
 * Settings is lazy, and every section under it with it.
 *
 * `docs/42` names the board, the list, the drawer and the palette as shell.
 * Administration is none of those: most people open it once, and the role editor
 * alone carries thirty permission rows. Splitting it keeps ADR-024's initial
 * budget for the surface people actually land on.
 */
const ProfileSettings = lazy(() =>
  import('./settings/ProfileSettings').then((m) => ({ default: m.ProfileSettings })),
)
const WorkspaceSettings = lazy(() =>
  import('./settings/WorkspaceSettings').then((m) => ({ default: m.WorkspaceSettings })),
)
const MembersSettings = lazy(() =>
  import('./settings/MembersSettings').then((m) => ({ default: m.MembersSettings })),
)
const TeamsSettings = lazy(() =>
  import('./settings/TeamsSettings').then((m) => ({ default: m.TeamsSettings })),
)
const RolesSettings = lazy(() =>
  import('./settings/RolesSettings').then((m) => ({ default: m.RolesSettings })),
)
const WorkflowSettings = lazy(() =>
  import('./settings/WorkflowSettings').then((m) => ({ default: m.WorkflowSettings })),
)
const EnvironmentsSettings = lazy(() =>
  import('./settings/EnvironmentsSettings').then((m) => ({ default: m.EnvironmentsSettings })),
)
const TagsSettings = lazy(() =>
  import('./settings/TagsSettings').then((m) => ({ default: m.TagsSettings })),
)

/**
 * The search parameters every route understands.
 *
 * # Why a filter lives in the URL and not in component state
 *
 * A filtered view is a *thing a person means* — "urgent bugs in Platform that
 * are overdue" — and the only way to hand that to a colleague, bookmark it, or
 * come back to it after a reload is for it to be the address. Filters kept in
 * `useState` are lost on refresh, cannot be linked, and quietly disagree between
 * the board and the list when a user switches views expecting the same scope.
 *
 * # Why the values are the server's own grammar
 *
 * `priority` here holds `HIGH,URGENT`, and `due` holds `<@today` — the exact
 * spellings `docs/27` §URL form defines, passed through untranslated. A client
 * dialect would be a second grammar to keep in step with the first, and `docs/27`
 * is explicit that there is one AST with two entry points. The visible cost is
 * that the address bar shows `due=%3C%40today`; the benefit is that a filter that
 * works in the UI works in `curl`, and neither can drift from the other.
 */
export interface AppSearch {
  /** The task open in the drawer, if any. */
  readonly task?: string
  /** The project a view is scoped to. Absent means "everything visible". */
  readonly project?: string
  /**
   * The owning team a view is scoped to (`docs/45`).
   *
   * A *scope*, like `project`, not a filter: it says which slice of the
   * workspace you are standing in, so "Clear filters" preserves it for the same
   * reason it preserves the project. Present-and-empty is the triage queue —
   * work owned by no team yet — which is why it survives validation.
   */
  readonly team?: string
  /** Free-text search, passed through to the server's `q` filter. */
  readonly q?: string
  /** Workflow status ids, comma-separated. */
  readonly status?: string
  /** `HIGH,URGENT` — the grammar's `in`. */
  readonly priority?: string
  /** `BUG,INCIDENT`. */
  readonly type?: string
  /** A user id, or `@me`, or empty for the grammar's `is_empty` (unassigned). */
  readonly assignee?: string
  /** A user id or `@me`. Distinct from `assignee`: who raised it, not who works it. */
  readonly reporter?: string
  /** A date clause: `<@today`, `<+7d`, `@today..+7d`. */
  readonly due?: string
  /** Permanent states, `BACKLOG,PLANNED` or `!COMPLETED,CANCELED` (`docs/23`). */
  readonly state?: string
  /** `created_at` / `updated_at` clauses, same spellings as `due`. */
  readonly created?: string
  readonly updated?: string
  /** Substring of the title — the grammar's `contains`. */
  readonly title?: string
  /** Tag ids, comma-separated, or empty for untagged. */
  readonly tag?: string
  /** `` (has no parent — top level) or `!` … see `tasks/query.ts`. */
  readonly parent?: string
  /** `true` to include archived tasks. The server defaults it to `false`. */
  readonly archived?: string
  /** `true` / `false`. Derived server-side from the dependency graph. */
  readonly blocked?: string
}

/** Every parameter above, so validation and clearing cannot drift from the type. */
export const SEARCH_KEYS = [
  'task',
  'project',
  'team',
  'q',
  'status',
  'priority',
  'type',
  'assignee',
  'reporter',
  'due',
  'state',
  'created',
  'updated',
  'title',
  'tag',
  'parent',
  'archived',
  'blocked',
] as const

/**
 * The filter parameters, as distinct from navigation state.
 *
 * `task` is not one — it says which drawer is open, not which rows to fetch — and
 * neither is `project` or `team`, which "Clear filters" deliberately preserves:
 * dropping the project would empty the board and unset the workflow, and
 * dropping the team would silently widen "my team's work" to the whole
 * workspace. Neither is what anyone means by clearing a filter.
 */
export const FILTER_KEYS = [
  'q',
  'status',
  'priority',
  'type',
  'assignee',
  'reporter',
  'due',
  'state',
  'created',
  'updated',
  'title',
  'tag',
  'parent',
  'archived',
  'blocked',
] as const

/**
 * Parameters whose empty value means `is_empty` rather than "no constraint".
 *
 * Exported because the *write* path needs the same list. `shell/navigation.ts`
 * prunes empty values out of the URL, and for a while it kept its own list of
 * one — so `?tag=` (untagged) and `?parent=` (top level) survived being typed
 * into the address bar but were dropped the moment any other control moved,
 * which is a filter that works until you touch the page. One list, two readers.
 */
export const EMPTY_IS_MEANINGFUL: ReadonlySet<string> = new Set([
  'assignee',
  'tag',
  'parent',
  'team',
])

/**
 * Validate rather than cast.
 *
 * TanStack Router hands the raw parsed query string here, which is attacker
 * controlled: a `task` that is an array or an object would reach a template
 * literal and produce a request to a path nobody wrote. Only strings survive.
 *
 * The empty string is kept for `assignee` alone, because `assignee=` is how the
 * grammar spells "unassigned" (`Operator::IsEmpty`) — dropping it the way every
 * other empty value is dropped would silently turn "show me unassigned work"
 * into "show me everything".
 */
function validateSearch(raw: Record<string, unknown>): AppSearch {
  const out: Record<string, string> = {}
  for (const key of SEARCH_KEYS) {
    const value = raw[key]
    if (typeof value !== 'string') continue
    // `assignee`, `tag` and `parent` all spell "unset" as a present-and-empty
    // value (`docs/27` §URL form: "`field=` — the empty value is how a URL says
    // 'unset'"). Dropping those the way every other empty value is dropped would
    // silently widen the filter instead of narrowing it.
    if (value === '' && !EMPTY_IS_MEANINGFUL.has(key)) continue
    out[key] = value
  }
  return out as AppSearch
}

const rootRoute = createRootRoute({ component: AppFrame, validateSearch })

const listRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: TaskListView,
})
const boardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/board',
  component: BoardView,
})
/**
 * The full task surface.
 *
 * A path, not a search parameter, because `design/LAYOUT-AND-INTERACTION-GUIDELINES.md`
 * §4 makes it the *default* detail surface rather than an overlay: it replaces
 * the view rather than sitting over one, it is what a pasted link and a new tab
 * get, and it is the only shape with the width the specification requires. The
 * peek keeps `?task=` — that one is an overlay and must preserve the view under
 * it.
 */
const taskRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/tasks/$taskId',
  component: function TaskRoute(): ReactElement {
    const { taskId } = taskRoute.useParams()
    return <TaskDetail taskId={taskId} />
  },
})

/**
 * Home answers "whose turn is it" (`docs/45`); My Work answers "what is
 * assigned, reported by, or overdue for me". They overlap and are not the same
 * question, so both exist and Home is the one the rail points at first.
 */
const homeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/home',
  component: HomeView,
})
const myWorkRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/my-work',
  component: MyWorkView,
})
const environmentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/environments',
  component: function EnvironmentsRoute(): ReactElement {
    return (
      <Suspense
        fallback={
          <p className="empty" role="status">
            Loading environments…
          </p>
        }
      >
        <EnvironmentView />
      </Suspense>
    )
  },
})
const reportsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/reports',
  component: function ReportsRoute(): ReactElement {
    return (
      <Suspense fallback={<p className="empty">Loading reports…</p>}>
        <ReportsView />
      </Suspense>
    )
  },
})

/**
 * Exported so a test can build its own router over the same tree.
 *
 * `createRouter` produces a singleton the module augmentation below types the
 * whole app against; a test that reused it would share history between cases and
 * see the previous test's route.
 */
/**
 * Settings, as a route tree.
 *
 * A layout route with children rather than one component switching on a
 * parameter: each section is a separate address someone can be sent to — "your
 * admin can change that under Roles" is only useful if Roles has a URL — and the
 * router, not the component, is what makes the back button work between them.
 */
const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  component: SettingsLayout,
})

/** Bare `/settings` lands on the one section every signed-in person can use. */
const settingsIndexRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: '/',
  component: function SettingsIndex(): ReactElement {
    return <Lazy label="profile settings">{<ProfileSettings />}</Lazy>
  },
})

function settingsChild(path: string, label: string, element: ReactElement): AnyRoute {
  return createRoute({
    getParentRoute: () => settingsRoute,
    path,
    component: function SettingsSection(): ReactElement {
      return <Lazy label={label}>{element}</Lazy>
    },
  })
}

/**
 * One fallback wording for every lazy section.
 *
 * `role="status"` rather than a bare paragraph: the panel is being replaced
 * under a navigation the user just made, and a screen-reader user otherwise
 * hears nothing between the click and the content.
 */
function Lazy({ label, children }: { label: string; children: ReactElement }): ReactElement {
  return (
    <Suspense fallback={<p className="empty" role="status">{`Loading ${label}…`}</p>}>
      {children}
    </Suspense>
  )
}

const settingsChildren = [
  settingsIndexRoute,
  settingsChild('/profile', 'profile settings', <ProfileSettings />),
  settingsChild('/workspace', 'workspace settings', <WorkspaceSettings />),
  settingsChild('/members', 'members', <MembersSettings />),
  settingsChild('/teams', 'teams', <TeamsSettings />),
  settingsChild('/roles', 'roles', <RolesSettings />),
  settingsChild('/workflow', 'the workflow', <WorkflowSettings />),
  settingsChild('/environments', 'environments', <EnvironmentsSettings />),
  settingsChild('/tags', 'tags', <TagsSettings />),
]

export const routeTree = rootRoute.addChildren([
  listRoute,
  boardRoute,
  homeRoute,
  myWorkRoute,
  taskRoute,
  reportsRoute,
  environmentsRoute,
  settingsRoute.addChildren(settingsChildren),
])

export const router = createRouter({ routeTree })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
