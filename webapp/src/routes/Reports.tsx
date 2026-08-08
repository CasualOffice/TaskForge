/**
 * Reports are lazy-always (docs/42 §What is not — lazy, always). This module
 * exists so the build produces at least one dynamic chunk and the report can
 * demonstrate that initial and lazy bytes are actually separated.
 */
import type { ReactElement } from 'react'

export default function Reports(): ReactElement {
  return <p>Reports render here, in a chunk the shell never downloads.</p>
}
