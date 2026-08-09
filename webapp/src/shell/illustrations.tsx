/**
 * Small original SVG scenes for operational empty states (`docs/54`).
 * They contain no text or meaning unavailable in the adjacent copy.
 */
import type { ReactElement } from 'react'

const SVG = {
  viewBox: '0 0 160 120',
  fill: 'none',
  xmlns: 'http://www.w3.org/2000/svg',
  focusable: 'false',
  'aria-hidden': true,
} as const

export function BlankSlateIllustration(): ReactElement {
  return (
    <svg {...SVG} className="empty-illustration" role="img">
      <path className="empty-illustration__wash" d="M24 89c0-27 20-50 49-50 25 0 30 17 48 17 12 0 21 10 21 23 0 19-15 27-38 27H53c-18 0-29-6-29-17Z" />
      <rect className="empty-illustration__paper" x="43" y="24" width="72" height="76" rx="10" />
      <path className="empty-illustration__line" d="M60 47h37M60 62h28M60 77h20" />
      <circle className="empty-illustration__primary" cx="111" cy="88" r="19" />
      <path className="empty-illustration__check" d="m102 88 6 6 12-14" />
      <path className="empty-illustration__spark" d="M126 27v9M122 31.5h8M31 59v7M27.5 62.5h7" />
    </svg>
  )
}

export function RecoveryIllustration(): ReactElement {
  return (
    <svg {...SVG} className="empty-illustration" role="img">
      <path className="empty-illustration__wash" d="M20 87c0-22 18-39 40-39 12 0 20 6 29 6 12 0 19-11 32-11 17 0 29 13 29 30 0 21-16 33-39 33H54c-21 0-34-6-34-19Z" />
      <path className="empty-illustration__paper" d="M45 84a25 25 0 0 1 43-19l7 8" />
      <path className="empty-illustration__primary-stroke" d="m96 60-1 13-13-1" />
      <path className="empty-illustration__paper" d="M113 73a25 25 0 0 1-43 19l-7-8" />
      <path className="empty-illustration__primary-stroke" d="m62 97 1-13 13 1" />
      <circle className="empty-illustration__primary" cx="80" cy="32" r="12" />
      <path className="empty-illustration__check" d="M80 26v7M80 38h.01" />
    </svg>
  )
}

/** Decorative product scene for the sign-in story panel. */
export function SignInIllustration(): ReactElement {
  return (
    <svg
      viewBox="0 0 560 340"
      className="signin-illustration"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      focusable="false"
      aria-hidden="true"
    >
      <path className="signin-illustration__wash" d="M38 286c0-82 66-149 148-149 50 0 76 26 114 26 53 0 75-57 132-57 62 0 102 48 102 106 0 73-55 112-148 112H146c-67 0-108-11-108-38Z" />
      <rect className="signin-illustration__window" x="74" y="42" width="412" height="250" rx="18" />
      <path className="signin-illustration__divider" d="M74 88h412M164 88v204" />
      <circle className="signin-illustration__dot" cx="98" cy="65" r="5" />
      <circle className="signin-illustration__dot" cx="116" cy="65" r="5" />
      <circle className="signin-illustration__dot" cx="134" cy="65" r="5" />
      <rect className="signin-illustration__nav-active" x="92" y="111" width="54" height="28" rx="7" />
      <path className="signin-illustration__nav-line" d="M97 160h39M97 184h31M97 208h43" />
      <rect className="signin-illustration__card" x="188" y="111" width="126" height="68" rx="10" />
      <rect className="signin-illustration__card" x="334" y="111" width="126" height="68" rx="10" />
      <rect className="signin-illustration__card" x="188" y="198" width="126" height="68" rx="10" />
      <rect className="signin-illustration__card" x="334" y="198" width="126" height="68" rx="10" />
      <path className="signin-illustration__card-line" d="M207 133h61M207 149h85M353 133h67M353 149h46M207 220h54M207 236h81M353 220h73M353 236h52" />
      <circle className="signin-illustration__avatar" cx="291" cy="160" r="8" />
      <circle className="signin-illustration__avatar" cx="438" cy="160" r="8" />
      <circle className="signin-illustration__avatar" cx="291" cy="247" r="8" />
      <path className="signin-illustration__primary" d="m451 275 12 12 26-31" />
    </svg>
  )
}

export function ReportsIllustration(): ReactElement {
  return (
    <svg {...SVG} className="empty-illustration" role="img">
      <path className="empty-illustration__wash" d="M18 91c0-27 22-49 49-49 18 0 25 10 39 10 17 0 25-16 38-7 10 7 13 20 10 32-5 20-19 29-47 29H51c-21 0-33-5-33-15Z" />
      <rect className="empty-illustration__paper" x="35" y="24" width="91" height="76" rx="10" />
      <path className="empty-illustration__line" d="M53 81V65M73 81V51M93 81V59M113 81V39" />
      <path className="empty-illustration__primary-stroke" d="m49 55 22-15 21 8 24-22" />
      <circle className="empty-illustration__primary" cx="116" cy="26" r="5" />
    </svg>
  )
}
