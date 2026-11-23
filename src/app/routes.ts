// Destinations (hash routes, so links and the back button work without a server router).
import { useCallback, useEffect, useState } from 'react';

export type Route = 'home' | 'connect' | 'air' | 'lens' | 'rewind' | 'voice' | 'settings';
export const ROUTES: Route[] = ['home', 'connect', 'air', 'lens', 'rewind', 'voice', 'settings'];

export function parseRoute(hash: string): Route {
  const r = hash.replace(/^#\/?/, '').split(/[/?]/)[0];
  return (ROUTES as string[]).includes(r) ? (r as Route) : 'home';
}

export function useHashRoute(): [Route, (r: Route) => void] {
  const [route, setRoute] = useState<Route>(() => (typeof window === 'undefined' ? 'home' : parseRoute(window.location.hash)));
  useEffect(() => {
    const on = () => setRoute(parseRoute(window.location.hash));
    window.addEventListener('hashchange', on);
    return () => window.removeEventListener('hashchange', on);
  }, []);
  const go = useCallback((r: Route) => {
    const hash = r === 'home' ? '#/' : `#/${r}`;
    if (window.location.hash !== hash) window.location.hash = hash;
    setRoute(r);
    window.scrollTo?.(0, 0);
  }, []);
  return [route, go];
}
