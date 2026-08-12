/**
 * The palette's people commands.
 *
 * # The failure these tests prevent
 *
 * The shell's search button has read "Search tasks, projects and people" since
 * it was written, and people were the one third of that sentence nothing
 * fetched. The palette searched tasks and projects; typing a colleague's name
 * returned whatever tasks happened to carry it in the search document — which,
 * because the projection indexes the *reporter's* display name in weight B,
 * meant every task that person had ever raised, with nothing on screen to say
 * why.
 *
 * So what is asserted here is the promise, not the implementation: a person's
 * name reaches a command, choosing it filters by assignee, and an anonymized
 * account (ADR-026, `email: null`) does not put the string "null" in front of
 * anybody.
 */
import { describe, expect, it, vi } from 'vitest'

import { rank } from './commands'
import { buildCommands, type PaletteContext } from './registry'

function context(overrides: Partial<PaletteContext> = {}): PaletteContext {
  return {
    term: '',
    projects: [],
    scopedProject: undefined,
    workspaces: [],
    people: [],
    mayCreateTask: false,
    go: () => undefined,
    setProject: () => undefined,
    setAssignee: () => undefined,
    chooseWorkspace: () => undefined,
    createTask: () => undefined,
    clearFilters: () => undefined,
    ...overrides,
  }
}

const ASH = { id: 'u-1', name: 'Ash Bekele', email: 'ash@example.test' }

describe('people in the palette', () => {
  it('offers a command per member', () => {
    const commands = buildCommands(context({ people: [ASH] }))
    const person = commands.find((command) => command.id === 'person-u-1')
    expect(person?.title).toBe('Work assigned to Ash Bekele')
  })

  it('finds a person by the name somebody would type', () => {
    const commands = buildCommands(context({ people: [ASH] }))
    const titles = rank(commands, 'Ash Bekele').map((command) => command.title)
    expect(titles).toContain('Work assigned to Ash Bekele')
  })

  it('finds a person by email, which is how you search a name you cannot spell', () => {
    const commands = buildCommands(context({ people: [ASH] }))
    const titles = rank(commands, 'ash@example').map((command) => command.title)
    expect(titles).toContain('Work assigned to Ash Bekele')
  })

  it('filters by assignee rather than pretending there is a person page', () => {
    // The failure this forbids: navigating somewhere that does not exist. There
    // is no profile route, so the only honest thing a person command can do is
    // answer "what is this person working on".
    const setAssignee = vi.fn()
    const commands = buildCommands(context({ people: [ASH], setAssignee }))
    commands.find((command) => command.id === 'person-u-1')?.run()
    expect(setAssignee).toHaveBeenCalledWith('u-1')
  })

  it('does not print "null" for an anonymized account', () => {
    // ADR-026 nulls the email on anonymization. Interpolating it straight into
    // the keyword string is how "null" becomes a searchable term that matches
    // every anonymized person at once.
    const commands = buildCommands(
      context({ people: [{ id: 'u-2', name: 'Former colleague', email: null }] }),
    )
    const person = commands.find((command) => command.id === 'person-u-2')
    expect(person?.keywords).not.toContain('null')
    expect(rank(commands, 'null').map((c) => c.id)).not.toContain('person-u-2')
  })

  it('adds nothing when the workspace has no members loaded yet', () => {
    const commands = buildCommands(context())
    expect(commands.filter((command) => command.id.startsWith('person-'))).toHaveLength(0)
  })
})
