// Appearance preference: system (default), light or dark. A display
// preference only — stored locally, never anything about conversations.

export type ThemePref = 'system' | 'light' | 'dark';
const KEY = 'talktual.theme.v1';

export function getThemePref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    return v === 'light' || v === 'dark' ? v : 'system';
  } catch {
    return 'system';
  }
}

export function applyTheme(pref: ThemePref): void {
  const el = document.documentElement;
  if (pref === 'system') el.removeAttribute('data-theme');
  else el.setAttribute('data-theme', pref);
}

export function setThemePref(pref: ThemePref): void {
  try {
    if (pref === 'system') localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, pref);
  } catch {
    /* applies for this page only */
  }
  applyTheme(pref);
}
