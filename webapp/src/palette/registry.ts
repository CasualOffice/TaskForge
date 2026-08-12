/**
 * Turning registry contributions into commands the palette can run.
 *
 * # The failure this module prevents
 *
 * A palette with a hard-coded array. `docs/34` and C-017 put the core's own
 * commands in the extension point registry so the seam is exercised before a
 * plugin depends on it; docs/42 makes the palette the reason navigation can stay
 * at seven items. Both promises are worth nothing if the palette's contents are
 * a literal in a component — the registry becomes documentation, and the first
 * contributed command has nowhere to appear.
 *
 * So the entries come from `contributions('ui.command')` and
 * `contributions('ui.project.tab')`, and this file is the *binding*: what each
 * declared slug actually does in this client.
 *
 * # A declared command with no implementation is listed, disabled, with a reason
 *
 * `assign` and `transition` are registered by the core and need a task to act
 * on, which the palette has no notion of yet. Dropping them would hide the fact
 * that the registry declares them; running them would be a lie. They appear
 * greyed with the reason, which is the only honest third option.
 */
import { contributions } from '../extensions/coreContributions'
import type { Command } from './commands'

/** Everything the bindings need from the app, passed in rather than imported. */
export interface PaletteContext {
  readonly term: string
  readonly projects: readonly { id: string; key: string; name: string }[]
  readonly scopedProject: string | undefined
  readonly workspaces: readonly { id: string; name: string; slug: string }[]
  /**
   * Workspace members, for the "and people" the header has always promised.
   *
   * They are people to *find work by*, not profiles to open — there is no
   * person page to go to, so a command that claimed to open one would be the
   * kind of dead end this file's header calls a lie. Choosing one filters the
   * task list to their work.
   */
  readonly people: readonly { id: string; name: string; email: string | null }[]
  readonly mayCreateTask: boolean
  readonly go: (to: string) => void
  readonly setProject: (id: string | undefined) => void
  readonly setAssignee: (id: string) => void
  readonly chooseWorkspace: (id: string) => void
  readonly createTask: (projectId: string, title: string) => void
  readonly clearFilters: () => void
}

/** The route each `ui.project.tab` slug navigates to in this client. */
const TAB_ROUTES: Readonly<Record<string, string>> = {
  board: '/board',
  list: '/',
  reports: '/reports',
}

/**
 * The settings sections, as commands.
 *
 * The keywords are what people actually type — "password", "invite", "who can"
 * — rather than the section's own name, which they would have to know first.
 */
const SETTINGS_SECTIONS: ReadonlyArray<{ slug: string; title: string; keywords: string }> = [
  { slug: 'profile', title: 'Your profile', keywords: 'password time zone sessions account me' },
  { slug: 'workspace', title: 'Workspace', keywords: 'rename slug admin' },
  { slug: 'members', title: 'Members', keywords: 'people invite invitation remove who' },
  { slug: 'teams', title: 'Teams', keywords: 'group squad' },
  { slug: 'roles', title: 'Roles', keywords: 'permissions grant revoke who can access' },
  { slug: 'workflow', title: 'Workflow', keywords: 'statuses transitions states columns' },
  { slug: 'tags', title: 'Tags', keywords: 'labels vocabulary' },
]

export function buildCommands(context: PaletteContext): readonly Command[] {
  const commands: Command[] = []

  // Navigation, from the project-tab contributions. My Work is not a project
  // tab — it spans projects — so it is added beside them rather than pretended
  // into a point it does not belong to.
  commands.push({
    id: 'go-my-work',
    title: 'Go to My Work',
    group: 'Go',
    keywords: 'mine assigned overdue',
    run: () => context.go('/my-work'),
  })
  for (const tab of contributions('ui.project.tab')) {
    const route = TAB_ROUTES[tab.slug]
    if (route === undefined) continue
    commands.push({
      id: `tab-${tab.slug}`,
      title: `Go to ${tab.title}`,
      group: 'Go',
      keywords: `${tab.slug} view`,
      run: () => context.go(route),
    })
  }

  // Settings, one command per section. Seven entries rather than one "Go to
  // settings", because the thing a person is looking for is "roles" or "my
  // password" — a command that lands them on a menu they then have to read is a
  // second navigation, and the palette exists to remove the first one.
  for (const section of SETTINGS_SECTIONS) {
    commands.push({
      id: `settings-${section.slug}`,
      title: `Settings: ${section.title}`,
      group: 'Go',
      keywords: section.keywords,
      run: () => context.go(`/settings/${section.slug}`),
    })
  }

  for (const command of contributions('ui.command')) {
    commands.push(...bind(command.slug, command.title, context))
  }

  for (const workspace of context.workspaces) {
    commands.push({
      id: `ws-${workspace.id}`,
      title: `Switch to ${workspace.name}`,
      group: 'Go',
      keywords: `workspace ${workspace.slug}`,
      run: () => context.chooseWorkspace(workspace.id),
    })
  }

  // People. Beside the workspaces rather than behind a `ui.command` slug,
  // because the registry declares no `go-to-person` contribution and inventing
  // one here would put a slug in the palette that `docs/34` never registered.
  //
  // The email is a keyword and not part of the title: it is how somebody
  // searches for a colleague whose display name they cannot spell, and it is
  // not something to paint across a list that may be on a shared screen. It is
  // `null` once an account is anonymized (ADR-026), which is exactly when it
  // must not be shown.
  for (const person of context.people) {
    commands.push({
      id: `person-${person.id}`,
      title: `Work assigned to ${person.name}`,
      group: 'Go',
      keywords: `person people assignee who ${person.email ?? ''}`,
      run: () => context.setAssignee(person.id),
    })
  }

  commands.push({
    id: 'view-clear',
    title: 'Clear filters',
    group: 'View',
    keywords: 'reset all projects',
    run: context.clearFilters,
  })

  return commands
}

/** What one declared `ui.command` slug becomes here. May be several, or none. */
function bind(slug: string, title: string, context: PaletteContext): Command[] {
  if (slug === 'create-task') {
    const draft = context.term.trim()
    const project = context.scopedProject
    if (!context.mayCreateTask) {
      return [unavailable(slug, title, 'You do not have permission to create tasks here.')]
    }
    if (project === undefined) {
      return [unavailable(slug, title, 'Choose a project first — a task belongs to one.')]
    }
    if (draft === '') {
      return [unavailable(slug, title, 'Type the title, then choose this.')]
    }
    return [
      {
        id: 'create-task',
        title: `Create task “${draft}”`,
        group: 'Task',
        keywords: 'new add',
        run: () => context.createTask(project, draft),
      },
    ]
  }

  if (slug === 'go-to-project') {
    return context.projects.map((project) => ({
      id: `project-${project.id}`,
      title: `Go to ${project.key} — ${project.name}`,
      group: 'Go',
      keywords: `project ${project.key}`,
      run: () => context.setProject(project.id),
    }))
  }

  if (slug === 'search') {
    // Implemented as the palette's own behaviour: typing searches tasks. Listed
    // anyway so the registry's `search` contribution is visibly bound to
    // something rather than quietly absent.
    return [
      {
        id: 'search',
        title: 'Search tasks',
        group: 'Task',
        keywords: 'find query q',
        hint: 'type',
        run: () => undefined,
      },
    ]
  }

  // `assign` and `transition` act on a task, which the palette has no notion of.
  return [unavailable(slug, title, 'Open a task first — this acts on one.')]
}

function unavailable(slug: string, title: string, because: string): Command {
  return {
    id: `unavailable-${slug}`,
    title,
    group: 'Task',
    unavailable: because,
    run: () => undefined,
  }
}
