/** Exact report values, used where a chart would obscure the answer. */
import type { ReactElement } from 'react'

import type { Point } from './charts'

export function TableChart({
  points,
  caption,
  dimension,
  measure,
}: {
  points: readonly Point[]
  caption: string
  dimension: string
  measure: string
}): ReactElement {
  return (
    <table className="chart__table">
      <caption className="visually-hidden">{caption}</caption>
      <thead>
        <tr>
          <th scope="col">{dimension}</th>
          <th scope="col">{measure}</th>
        </tr>
      </thead>
      <tbody>
        {points.map((point) => (
          <tr key={point.label}>
            <th scope="row">{point.label}</th>
            <td>{point.formatted}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
