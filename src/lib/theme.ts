import type { PaletteChoice, ThemeChoice } from '../types'


export type ThemeEffective = 'light' | 'dark'
export const THEME_STORAGE_KEY = 'scribe.theme'
export const PALETTE_DEFAULT: PaletteChoice = 'graphite'
const SYSTEM_MEDIA_QUERY = '(prefers-color-scheme: light)'


export function system_media_query(): MediaQueryList | null {
    if (typeof window === 'undefined') return null
    if (typeof window.matchMedia !== 'function') return null

    return window.matchMedia(SYSTEM_MEDIA_QUERY)
}

export function theme_resolve(choice: ThemeChoice | null): ThemeEffective {
    if (choice === 'light') return 'light'
    if (choice === 'dark') return 'dark'

    const query = system_media_query()

    if (query === null) return 'dark'

    return query.matches ? 'light' : 'dark'
}

export function theme_storage_read(): ThemeChoice | null {
    if (typeof localStorage === 'undefined') return null

    const stored = localStorage.getItem(THEME_STORAGE_KEY)

    if (stored === 'light') return 'light'
    if (stored === 'dark') return 'dark'

    return null
}

export function theme_storage_write(choice: ThemeChoice | null) {
    if (typeof localStorage === 'undefined') return

    switch (choice) {
        case 'light':
        case 'dark':
            localStorage.setItem(THEME_STORAGE_KEY, choice)

            return
        default:
            localStorage.removeItem(THEME_STORAGE_KEY)
    }
}

export function theme_apply_to_html(effective: ThemeEffective) {
    if (typeof document === 'undefined') return

    const root = document.documentElement

    root.classList.toggle('theme-light', effective === 'light')
    root.classList.toggle('theme-dark', effective === 'dark')
    root.style.colorScheme = effective
}

export function palette_apply_to_html(palette: PaletteChoice) {
    if (typeof document === 'undefined') return

    document.documentElement.dataset['palette'] = palette
}
