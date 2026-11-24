// Talktual icon set — drawn for this app (replaces the generic icon library).
//
// One 24-unit grid, a hairline 1.6 stroke with round terminals, generous
// negative space, and a faint duotone layer (the shape's body at low opacity)
// for depth, like hierarchical symbols. No sparkles, no robots, no "AI" glyphs.
// Exports keep the previous names so call sites read the same.

import type { ReactNode, SVGProps } from 'react';

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, 'children'> {
  strokeWidth?: number | string;
}
export type IconComponent = (props: IconProps) => ReactNode;

const TONE = 0.16;

function icon(name: string, draw: ReactNode, tone?: ReactNode): IconComponent {
  const Icon = ({ className, strokeWidth = 1.6, ...rest }: IconProps) => {
    const labelled = 'aria-label' in rest || 'aria-labelledby' in rest;
    return (
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        width={24}
        height={24}
        fill="none"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
        className={className}
        data-icon={name}
        {...(labelled ? { role: 'img' } : { 'aria-hidden': true })}
        {...rest}
      >
        {tone && (
          <g fill="currentColor" fillOpacity={TONE} stroke="none">
            {tone}
          </g>
        )}
        {draw}
      </svg>
    );
  };
  Icon.displayName = name;
  return Icon;
}

// ---- Navigation -----------------------------------------------------------

/** Home: a house with an arched door. */
export const House = icon(
  'home',
  <path d="M4 10.4 12 4l8 6.4v8.35A1.75 1.75 0 0 1 18.25 20.5H15.5v-4.75a3.5 3.5 0 0 0-7 0v4.75H5.75A1.75 1.75 0 0 1 4 18.75z" />,
  <path d="M4 10.4 12 4l8 6.4v8.35A1.75 1.75 0 0 1 18.25 20.5H5.75A1.75 1.75 0 0 1 4 18.75z" />
);

/** Connect: two voices in conversation — one in front, one answering behind. */
export const PhoneCall = icon(
  'connect',
  <>
    <path d="M3.5 7A2.75 2.75 0 0 1 6.25 4.25h6.5A2.75 2.75 0 0 1 15.5 7v3.5a2.75 2.75 0 0 1-2.75 2.75H9L5.5 15.75v-2.6A2.75 2.75 0 0 1 3.5 10.5z" />
    <path d="M15.5 9.25h2.75a2.25 2.25 0 0 1 2.25 2.25v3.75a2.5 2.5 0 0 1-1.75 2.39v2.11l-3-2.25H13a2.5 2.5 0 0 1-2.29-1.5V13.5" />
  </>,
  <path d="M3.5 7A2.75 2.75 0 0 1 6.25 4.25h6.5A2.75 2.75 0 0 1 15.5 7v3.5a2.75 2.75 0 0 1-2.75 2.75H9L5.5 15.75v-2.6A2.75 2.75 0 0 1 3.5 10.5z" />
);

/** AIR: a listener at the centre, the room's sound arriving from both sides. */
export const Ear = icon(
  'air',
  <>
    <path d="M8.6 8.6a4.8 4.8 0 0 0 0 6.8" />
    <path d="M15.4 8.6a4.8 4.8 0 0 1 0 6.8" />
    <path d="M5.75 5.75a8.85 8.85 0 0 0 0 12.5" />
    <path d="M18.25 5.75a8.85 8.85 0 0 1 0 12.5" />
    <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
  </>,
  <circle cx="12" cy="12" r="4.2" />
);

/** Lens: a viewfinder framing two lines of text. */
export const ScanText = icon(
  'lens',
  <>
    <path d="M4 8.25V6.75A2.75 2.75 0 0 1 6.75 4h1.5" />
    <path d="M15.75 4h1.5A2.75 2.75 0 0 1 20 6.75v1.5" />
    <path d="M20 15.75v1.5A2.75 2.75 0 0 1 17.25 20h-1.5" />
    <path d="M8.25 20h-1.5A2.75 2.75 0 0 1 4 17.25v-1.5" />
    <path d="M8.5 10.25h7" />
    <path d="M8.5 13.75h4.5" />
  </>,
  <rect x="6.75" y="6.75" width="10.5" height="10.5" rx="2.25" />
);

/** Rewind: time turning back. */
export const History = icon(
  'rewind',
  <>
    <path d="M5.4 8.4A7.75 7.75 0 1 1 4.25 12" />
    <path d="M5.25 4.75v3.75H9" />
    <path d="M12 8.25v4l2.75 1.75" />
  </>,
  <circle cx="12" cy="12" r="7.75" />
);

/** Voice: a voice print. */
export const AudioLines = icon(
  'voice',
  <>
    <path d="M4 10.5v3" />
    <path d="M8 7.5v9" />
    <path d="M12 4.5v15" />
    <path d="M16 8v8" />
    <path d="M20 10.5v3" />
  </>
);

/** Settings: a precise eight-tooth gear. */
export const Settings = icon(
  'settings',
  <>
    <path d="M10.07 5.01 10.55 3.02 13.45 3.02 13.93 5.01 15.57 5.69 17.33 4.62 19.38 6.67 18.31 8.43 18.99 10.07 20.98 10.55 20.98 13.45 18.99 13.93 18.31 15.57 19.38 17.33 17.33 19.38 15.57 18.31 13.93 18.99 13.45 20.98 10.55 20.98 10.07 18.99 8.43 18.31 6.67 19.38 4.62 17.33 5.69 15.57 5.01 13.93 3.02 13.45 3.02 10.55 5.01 10.07 5.69 8.43 4.62 6.67 6.67 4.62 8.43 5.69Z" />
    <circle cx="12" cy="12" r="2.75" />
  </>,
  <path d="M10.07 5.01 10.55 3.02 13.45 3.02 13.93 5.01 15.57 5.69 17.33 4.62 19.38 6.67 18.31 8.43 18.99 10.07 20.98 10.55 20.98 13.45 18.99 13.93 18.31 15.57 19.38 17.33 17.33 19.38 15.57 18.31 13.93 18.99 13.45 20.98 10.55 20.98 10.07 18.99 8.43 18.31 6.67 19.38 4.62 17.33 5.69 15.57 5.01 13.93 3.02 13.45 3.02 10.55 5.01 10.07 5.69 8.43 4.62 6.67 6.67 4.62 8.43 5.69Z" />
);

/** Adjust: two sliders. */
export const SlidersHorizontal = icon(
  'adjust',
  <>
    <path d="M4 7.5h8.5" />
    <path d="M17.5 7.5H20" />
    <circle cx="15" cy="7.5" r="2.25" />
    <path d="M4 16.5h2.5" />
    <path d="M11.5 16.5H20" />
    <circle cx="9" cy="16.5" r="2.25" />
  </>,
  <>
    <circle cx="15" cy="7.5" r="2.25" />
    <circle cx="9" cy="16.5" r="2.25" />
  </>
);

// ---- Direction ------------------------------------------------------------

export const ChevronRight = icon('chevron-right', <path d="m9.5 6 6 6-6 6" />);
export const ChevronLeft = icon('chevron-left', <path d="m14.5 6-6 6 6 6" />);
export const ArrowRight = icon(
  'arrow-right',
  <>
    <path d="M4.75 12h14.5" />
    <path d="m13.5 6.25 5.75 5.75-5.75 5.75" />
  </>
);
export const ArrowLeft = icon(
  'arrow-left',
  <>
    <path d="M19.25 12H4.75" />
    <path d="M10.5 6.25 4.75 12l5.75 5.75" />
  </>
);
export const ArrowUpRight = icon(
  'arrow-up-right',
  <>
    <path d="M7 17 17 7" />
    <path d="M8.75 7H17v8.25" />
  </>
);

// ---- Actions --------------------------------------------------------------

export const X = icon(
  'close',
  <>
    <path d="m6.5 6.5 11 11" />
    <path d="m17.5 6.5-11 11" />
  </>
);
export const Check = icon('check', <path d="m5 12.5 4.5 4.5L19 7.25" />);

export const Mic = icon(
  'mic',
  <>
    <rect x="9" y="3.25" width="6" height="11" rx="3" />
    <path d="M5.75 11.25a6.25 6.25 0 0 0 12.5 0" />
    <path d="M12 17.5v3.25" />
  </>,
  <rect x="9" y="3.25" width="6" height="11" rx="3" />
);
export const MicOff = icon(
  'mic-off',
  <>
    <path d="M15 9.5V6.25a3 3 0 0 0-5.65-1.41" />
    <path d="M9 9v2.25a3 3 0 0 0 4.9 2.32" />
    <path d="M5.75 11.25a6.25 6.25 0 0 0 10.5 4.58" />
    <path d="M18.25 11.25c0 .5-.06.98-.17 1.45" />
    <path d="M12 17.5v3.25" />
    <path d="m4 4 16 16" />
  </>
);

export const Keyboard = icon(
  'keyboard',
  <>
    <rect x="3" y="6" width="18" height="12" rx="3" />
    <path d="M7 10h.01M10.33 10h.01M13.67 10h.01M17 10h.01" strokeWidth={2.1} />
    <path d="M8.5 14h7" />
  </>,
  <rect x="3" y="6" width="18" height="12" rx="3" />
);

export const Square = icon('stop', <rect x="6.5" y="6.5" width="11" height="11" rx="2.75" />);

export const Play = icon(
  'play',
  <path d="M8.25 5.6v12.8a1 1 0 0 0 1.52.85l10.2-6.4a1 1 0 0 0 0-1.7L9.77 4.75a1 1 0 0 0-1.52.85z" />,
  <path d="M8.25 5.6v12.8a1 1 0 0 0 1.52.85l10.2-6.4a1 1 0 0 0 0-1.7L9.77 4.75a1 1 0 0 0-1.52.85z" />
);
export const Pause = icon(
  'pause',
  <>
    <rect x="6.25" y="5" width="3.75" height="14" rx="1.6" />
    <rect x="14" y="5" width="3.75" height="14" rx="1.6" />
  </>,
  <>
    <rect x="6.25" y="5" width="3.75" height="14" rx="1.6" />
    <rect x="14" y="5" width="3.75" height="14" rx="1.6" />
  </>
);

/** Play again: a turn back around a small play mark. */
export const RotateCcw = icon(
  'replay',
  <>
    <path d="M5.4 8.4A7.75 7.75 0 1 1 4.25 12" />
    <path d="M5.25 4.75v3.75H9" />
    <path d="M10.75 9.5v5l4-2.5z" />
  </>,
  <path d="M10.75 9.5v5l4-2.5z" />
);

const SPEAKER = 'M4 9.75v4.5c0 .55.45 1 1 1h2.6l4.15 3.6c.49.42 1.25.07 1.25-.57V5.72c0-.64-.76-.99-1.25-.57L7.6 8.75H5c-.55 0-1 .45-1 1z';
export const Volume2 = icon(
  'speaker',
  <>
    <path d={SPEAKER} />
    <path d="M16.25 9.25a3.9 3.9 0 0 1 0 5.5" />
    <path d="M18.9 6.6a7.6 7.6 0 0 1 0 10.8" />
  </>,
  <path d={SPEAKER} />
);
export const VolumeX = icon(
  'speaker-off',
  <>
    <path d={SPEAKER} />
    <path d="m16.25 9.75 4.5 4.5" />
    <path d="m20.75 9.75-4.5 4.5" />
  </>,
  <path d={SPEAKER} />
);

export const Trash2 = icon(
  'clear',
  <>
    <path d="M4.5 6.75h15" />
    <path d="M9.5 6.75V5.25c0-.83.67-1.5 1.5-1.5h2c.83 0 1.5.67 1.5 1.5v1.5" />
    <path d="m6.5 6.75.78 11.9a2 2 0 0 0 2 1.85h5.44a2 2 0 0 0 2-1.85l.78-11.9" />
    <path d="M10.25 10.75v5.5" />
    <path d="M13.75 10.75v5.5" />
  </>
);

export const Share = icon(
  'share',
  <>
    <path d="M8.25 9.75H7.5a2.25 2.25 0 0 0-2.25 2.25v6A2.25 2.25 0 0 0 7.5 20.25h9A2.25 2.25 0 0 0 18.75 18v-6a2.25 2.25 0 0 0-2.25-2.25h-.75" />
    <path d="M12 14.25V3.75" />
    <path d="M8.75 7 12 3.75 15.25 7" />
  </>
);

export const Copy = icon(
  'copy',
  <>
    <rect x="8.75" y="8.75" width="11.5" height="11.5" rx="2.5" />
    <path d="M15.25 8.75V6.25a2.5 2.5 0 0 0-2.5-2.5h-6.5a2.5 2.5 0 0 0-2.5 2.5v6.5a2.5 2.5 0 0 0 2.5 2.5h2.5" />
  </>,
  <rect x="8.75" y="8.75" width="11.5" height="11.5" rx="2.5" />
);

export const Search = icon(
  'search',
  <>
    <circle cx="10.75" cy="10.75" r="6.5" />
    <path d="m15.6 15.6 4.4 4.4" />
  </>,
  <circle cx="10.75" cy="10.75" r="6.5" />
);

/** End call: the hang-up handset. */
export const PhoneOff = icon(
  'end-call',
  <path d="M3.25 13.6c0-1.27.74-2.42 1.9-2.93C7.2 9.78 9.55 9.35 12 9.35s4.8.43 6.85 1.32a3.2 3.2 0 0 1 1.9 2.93v1.06c0 .74-.66 1.3-1.4 1.18l-2.67-.45a1.2 1.2 0 0 1-1-1.18v-1.73a12.4 12.4 0 0 0-7.36 0v1.73c0 .59-.43 1.09-1 1.18l-2.67.45c-.74.12-1.4-.44-1.4-1.18z" />,
  <path d="M3.25 13.6c0-1.27.74-2.42 1.9-2.93C7.2 9.78 9.55 9.35 12 9.35s4.8.43 6.85 1.32a3.2 3.2 0 0 1 1.9 2.93v1.06c0 .74-.66 1.3-1.4 1.18l-2.67-.45a1.2 1.2 0 0 1-1-1.18v-1.73a12.4 12.4 0 0 0-7.36 0v1.73c0 .59-.43 1.09-1 1.18l-2.67.45c-.74.12-1.4-.44-1.4-1.18z" />
);

/** Scan a code: a viewfinder with a scan line. */
export const ScanLine = icon(
  'scan',
  <>
    <path d="M4 8.25V6.75A2.75 2.75 0 0 1 6.75 4h1.5" />
    <path d="M15.75 4h1.5A2.75 2.75 0 0 1 20 6.75v1.5" />
    <path d="M20 15.75v1.5A2.75 2.75 0 0 1 17.25 20h-1.5" />
    <path d="M8.25 20h-1.5A2.75 2.75 0 0 1 4 17.25v-1.5" />
    <path d="M7 12h10" />
  </>
);

// ---- Status ---------------------------------------------------------------

export const Info = icon(
  'info',
  <>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 11v5" />
    <path d="M12 7.85h.01" strokeWidth={2.2} />
  </>,
  <circle cx="12" cy="12" r="8.5" />
);
export const AlertCircle = icon(
  'alert',
  <>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 7.75v5" />
    <path d="M12 16.15h.01" strokeWidth={2.2} />
  </>,
  <circle cx="12" cy="12" r="8.5" />
);
export const WifiOff = icon(
  'offline',
  <>
    <path d="M12 18.75h.01" strokeWidth={2.4} />
    <path d="M9 15.6a4.3 4.3 0 0 1 4.65-.9" />
    <path d="M5.75 12.35a8.9 8.9 0 0 1 3.4-2.1" />
    <path d="M14.9 10.25a8.9 8.9 0 0 1 3.35 2.1" />
    <path d="M2.75 9.1a13.3 13.3 0 0 1 3.1-2.25" />
    <path d="M10.6 5.3a13.3 13.3 0 0 1 10.65 3.8" />
    <path d="m4 4 16 16" />
  </>
);
export const Loader2 = icon('loading', <path d="M20.5 12A8.5 8.5 0 1 1 12 3.5" />);
