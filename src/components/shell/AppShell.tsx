import type { ReactNode } from 'react';
import { AudioLines, Ear, History, House, PhoneCall, ScanText, Settings } from 'lucide-react';
import type { Route } from '../../app/routes';
import { BrandMark } from '../Brand';
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

/** Navigation around pages; hidden during active sessions (AIR, a call). */
export function AppShell({ route, go, showNav, children }: { route: Route; go: (r: Route) => void; showNav: boolean; children: ReactNode }) {
  if (!showNav) return <>{children}</>;
  return (
    <div className="min-h-dvh lg:flex">
      <a href="#content" onClick={(e) => (e.preventDefault(), document.getElementById('content')?.focus())} className="sr-only z-50 rounded-md bg-raised text-body font-semibold focus:not-sr-only focus:fixed focus:left-3 focus:top-3 focus:px-4 focus:py-3">
        Skip to content
      </a>
      <DesktopRail route={route} go={go} />
      <div id="content" tabIndex={-1} className="min-w-0 flex-1 pb-[calc(64px+env(safe-area-inset-bottom))] outline-none lg:pb-0">
        {children}
      </div>
      <MobileBottomNav route={route} go={go} />
    </div>
  );
}

function NavItem({ d, active, go, layout }: { d: Dest; active: boolean; go: (r: Route) => void; layout: 'rail' | 'bar' }) {
  const Icon = d.icon;
  return (
    <a
      href={d.route === 'home' ? '#/' : `#/${d.route}`}
      onClick={(e) => {
        e.preventDefault();
        go(d.route);
      }}
      aria-current={active ? 'page' : undefined}
      className={cn(
        'flex flex-col items-center justify-center gap-1 rounded-md text-meta font-medium transition-colors duration-150',
        layout === 'rail' ? 'h-16 w-16' : 'h-14 min-w-0 flex-1',
        active ? 'text-fg' : 'text-fg-subtle hover:text-fg'
      )}
    >
      <span className={cn('flex h-7 w-11 items-center justify-center rounded-sm transition-colors', active && 'bg-accent-soft text-accent-text')}>
        <Icon className="h-5 w-5" aria-hidden="true" strokeWidth={active ? 2.2 : 1.8} />
      </span>
      {d.label}
    </a>
  );
}

function DesktopRail({ route, go }: { route: Route; go: (r: Route) => void }) {
  return (
    <nav aria-label="Main" className="sticky top-0 hidden h-dvh w-[88px] shrink-0 flex-col items-center border-r border-line py-4 lg:flex">
      <a href="#/" onClick={(e) => (e.preventDefault(), go('home'))} aria-label="Talktual home" className="mb-6 flex h-11 w-11 items-center justify-center rounded-md">
        <BrandMark size={18} />
      </a>
      <div className="flex flex-col gap-1">
        {PRIMARY.map((d) => (
          <NavItem key={d.route} d={d} active={route === d.route} go={go} layout="rail" />
        ))}
      </div>
      <div className="mt-auto flex flex-col gap-1">
        {SECONDARY.map((d) => (
          <NavItem key={d.route} d={d} active={route === d.route} go={go} layout="rail" />
        ))}
      </div>
    </nav>
  );
}

function MobileBottomNav({ route, go }: { route: Route; go: (r: Route) => void }) {
  return (
    <nav aria-label="Main" className="fixed inset-x-0 bottom-0 z-30 border-t border-line bg-bg pb-[env(safe-area-inset-bottom)] lg:hidden">
      <div className="mx-auto flex max-w-[560px] px-2">
        {PRIMARY.map((d) => (
          <NavItem key={d.route} d={d} active={route === d.route} go={go} layout="bar" />
        ))}
      </div>
    </nav>
  );
}

/** Standard page frame: one content column, page title, optional trailing action. */
export function Page({ title, description, action, children, wide }: { title: string; description?: ReactNode; action?: ReactNode; children: ReactNode; wide?: boolean }) {
  return (
    <main className={cn('mx-auto w-full px-4 pb-10 sm:px-6 lg:px-10', wide ? 'max-w-[880px]' : 'max-w-[720px]')}>
      <header className="flex min-h-16 items-end justify-between gap-3 pb-2 pt-[max(2.5rem,env(safe-area-inset-top))] lg:pt-14">
        <div className="min-w-0">
          <h1 className="text-title font-semibold tracking-[-0.01em]">{title}</h1>
          {description && <p className="mt-1.5 max-w-[34rem] text-body text-fg-muted">{description}</p>}
        </div>
        {action}
      </header>
      {children}
    </main>
  );
}
