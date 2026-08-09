/**
 * The live region every optimistic outcome is announced through.
 *
 * # The failure this module prevents
 *
 * An interface that only *looks* like it worked. docs/42 §Accessibility: "live
 * regions announce optimistic outcomes and errors." A card that slides to
 * another column is feedback for people who can see it slide; for everyone else
 * a drag either announces its result or produces silence indistinguishable from
 * a dropped event.
 *
 * One region for the whole app, not one per view: two `aria-live` regions
 * updating in the same tick race, and screen readers resolve that race by
 * dropping one of them.
 */
import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react'

type Announce = (message: string) => void

const AnnounceContext = createContext<Announce>(() => undefined)

export function Announcer({ children }: { children: ReactNode }): ReactElement {
  const [message, setMessage] = useState('')

  const announce = useCallback<Announce>((next) => {
    // Cleared first so an identical consecutive message still counts as a
    // change. Announcing "Moved to Active" twice in a row is otherwise silent
    // the second time, which is exactly when a user is checking whether their
    // second attempt worked.
    setMessage('')
    requestAnimationFrame(() => setMessage(next))
  }, [])

  const value = useMemo(() => announce, [announce])

  return (
    <AnnounceContext.Provider value={value}>
      {children}
      {/* `polite`, not `assertive`: these outcomes should not interrupt a user
          mid-sentence. `atomic` so the whole sentence is read, not the diff. */}
      <div className="visually-hidden" role="status" aria-live="polite" aria-atomic="true">
        {message}
      </div>
    </AnnounceContext.Provider>
  )
}

export function useAnnounce(): Announce {
  return useContext(AnnounceContext)
}
