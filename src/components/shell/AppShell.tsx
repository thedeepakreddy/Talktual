import type { ReactNode } from 'react';
import { motion } from 'motion/react';
import { AudioLines, Ear, History, House, PhoneCall, ScanText, Settings } from '../icons';
import type { Route } from '../../app/routes';
import { BrandMark } from '../Brand';
import { LiquidField, type FieldMood } from '../../three/LiquidField';
import { cn } from '../../utils';

interface Dest {
  route: Route;
  label: string;
  icon: typeof House;
}
/** The five primary destinations (approved). Voice Studio and Settings sit in the rail footer and on Home. */
export const PRIMARY: Dest[] = [
  { route: 'home', label: 'Home', icon: House },
  { route: 'connect', label: 'Connect', icon: PhoneCall },
  { route: 'air', label: 'AIR', icon: Ear },
  { route: 'lens', label: 'Lens', icon: ScanText },
  { route: 'rewind', label: 'Rewind', icon: History },
];
const SECONDARY: Dest[] = [
  { route: 'voice', label: 'Voice', icon: AudioLines },
  { route: 'settings', label: 'Settings', icon: Settings },
];

const MOOD: Record<Route, FieldMood> = { home: 'rich', air: 'session', connect: 'session', lens: 'calm', rewind: 'calm', voice: 'calm', settings: 'calm', engine: 'calm' };

/** Navigation around pages; hidden during active sessions (AIR, a call). The molten field sits behind everything. */
export function AppShell({ route, go, showNav, children }: { route: Route; go: (r: Route) => void; showNav: boolean; children: ReactNode }) {
  // The field stays mounted at the same position across pages and sessions:
  // one WebGL context for the app's lifetime, only its mood changes.
  return (
    <>
      <LiquidField mood={showNav ? MOOD[route] : 'session'} />
      {showNav ? (
        <div className="min-h-dvh">
          <a
            href="#content"
            onClick={(e) => (e.preventDefault(), document.getElementById('content')?.focus())}
            className="glass-strong sr-only z-50 rounded-full text-body font-semibold focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:px-5 focus:py-3"
          >
            Skip to content
          </a>
          <DesktopRail route={route} go={go} />
          <div id="content" tabIndex={-1} className="min-w-0 pb-[calc(96px+env(safe-area-inset-bottom))] outline-none lg:pb-0 lg:pl-[116px]">
            {children}
          </div>
          <TabBar route={route} go={go} />
        </div>
      ) : (
        children
      )}
    </>
  );
}

function href(r: Route) {
  return r === 'home' ? '#/' : `#/${r}`;
}

function RailItem({ d, active, go }: { d: Dest; active: boolean; go: (r: Route) => void }) {
  const Icon = d.icon;
  return (
    <a
      href={href(d.route)}
      onClick={(e) => {
        e.preventDefault();
        go(d.route);
      }}
      aria-current={active ? 'page' : undefined}
      className={cn('press relative flex h-[62px] w-[62px] flex-col items-center justify-center gap-1 rounded-[20px] text-[0.6875rem] font-semibold tracking-[0.01em]', active ? 'text-fg' : 'text-fg-subtle hover:text-fg')}
    >
      {active && <motion.span layoutId="rail-lens" className="glass-clear absolute inset-0 rounded-[20px]" transition={{ type: 'spring', stiffness: 420, damping: 34 }} />}
      <Icon className={cn('relative h-[22px] w-[22px]', active && 'text-accent-text')} aria-hidden="true" />
      <span className="relative">{d.label}</span>
    </a>
  );
}

function DesktopRail({ route, go }: { route: Route; go: (r: Route) => void }) {
  return (
    <nav aria-label="Main" className="glass-strong fixed bottom-3 left-3 top-3 z-30 hidden w-[88px] flex-col items-center rounded-[30px] py-4 lg:flex">
      <a href="#/" onClick={(e) => (e.preventDefault(), go('home'))} aria-label="Talktual home" className="press mb-5 flex h-12 w-12 items-center justify-center rounded-[16px]">
        <BrandMark size={17} />
      </a>
      <div className="flex flex-col gap-1.5">
        {PRIMARY.map((d) => (
          <RailItem key={d.route} d={d} active={route === d.route} go={go} />
        ))}
      </div>
      <div className="mt-auto flex flex-col gap-1.5">
        {SECONDARY.map((d) => (
          <RailItem key={d.route} d={d} active={route === d.route} go={go} />
        ))}
      </div>
    </nav>
  );
}

/** Floating liquid-glass tab bar (phones and tablets); a glass lens slides under the active tab. */
function TabBar({ route, go }: { route: Route; go: (r: Route) => void }) {
  return (
    <nav aria-label="Main" className="fixed inset-x-0 bottom-0 z-30 px-3 pb-[max(10px,env(safe-area-inset-bottom))] lg:hidden">
      <div className="glass-strong mx-auto flex h-[66px] max-w-[460px] items-stretch rounded-full p-1.5">
        {PRIMARY.map((d) => {
          const active = route === d.route;
          const Icon = d.icon;
          return (
            <a
              key={d.route}
              href={href(d.route)}
              onClick={(e) => {
                e.preventDefault();
                go(d.route);
              }}
              aria-current={active ? 'page' : undefined}
              className={cn('press relative flex min-w-0 flex-1 flex-col items-center justify-center gap-[3px] rounded-full text-[0.6875rem] font-semibold tracking-[0.01em]', active ? 'text-fg' : 'text-fg-subtle')}
            >
              {active && <motion.span layoutId="tab-lens" className="glass-clear absolute inset-0 rounded-full" style={{ background: 'linear-gradient(180deg, color-mix(in oklab, var(--accent) 22%, transparent), color-mix(in oklab, var(--accent) 8%, transparent))' }} transition={{ type: 'spring', stiffness: 460, damping: 36 }} />}
              <Icon className={cn('relative h-[22px] w-[22px]', active && 'text-accent-text')} aria-hidden="true" />
              <span className="relative max-w-full truncate px-0.5">{d.label}</span>
            </a>
          );
        })}
      </div>
    </nav>
  );
}

/** Standard page frame: one content column, a large title, optional eyebrow and trailing action. */
export function Page({
  title,
  eyebrow,
  description,
  action,
  children,
  wide,
}: {
  title: ReactNode;
  eyebrow?: string;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  wide?: boolean;
}) {
  return (
    <main className={cn('mx-auto w-full px-4 pb-12 sm:px-6 lg:px-10', wide ? 'max-w-[920px]' : 'max-w-[760px]')}>
      <header className="flex items-end justify-between gap-3 pb-6 pt-[max(3rem,calc(env(safe-area-inset-top)+1.5rem))] lg:pt-16">
        <div className="min-w-0">
          {eyebrow && <p className="t-eyebrow mb-2 text-accent-text">{eyebrow}</p>}
          <h1 className="t-title text-title lg:text-[2.375rem] lg:leading-[2.6rem]">{title}</h1>
          {description && <p className="mt-2.5 max-w-[36rem] text-body-lg text-fg-muted">{description}</p>}
        </div>
        {action}
      </header>
      {children}
    </main>
  );
}
