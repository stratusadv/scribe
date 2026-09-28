import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { THEME_STORAGE_KEY } from '../../src/lib/theme'


vi.mock('../../src/lib/ipc')

type ChangeListener = () => void

const change_listeners: ChangeListener[] = []

function match_media_install(matches: boolean) {
    const query = {
        matches,
        addEventListener: (_name: string, listener: ChangeListener) => {
            change_listeners.push(listener)
        },
    }

    vi.stubGlobal('matchMedia', vi.fn(() => query))

    return query
}

function flush() {
    return new Promise((resolve) => setTimeout(resolve, 0))
}

async function theme_import() {
    vi.resetModules()

    const [{ ipc }, { use_settings }, { use_theme }] = await Promise.all([
        import('../../src/lib/ipc'),
        import('../../src/composables/use_settings'),
        import('../../src/composables/use_theme'),
    ])

    vi.mocked(ipc.settings_update).mockResolvedValue(undefined)

    return { ipc, ...use_settings(), ...use_theme() }
}

beforeEach(() => {
    localStorage.clear()
    change_listeners.length = 0
    document.documentElement.className = ''
    document.documentElement.style.colorScheme = ''
    delete document.documentElement.dataset['palette']
})

afterEach(() => {
    vi.unstubAllGlobals()
})

describe('use_theme on load', () => {
    it('applies the system theme and the default palette immediately', async () => {
        match_media_install(true)

        const theme = await theme_import()

        expect(theme.theme_effective.value).toBe('light')
        expect(theme.palette_effective.value).toBe('graphite')
        expect(document.documentElement.classList.contains('theme-light')).toBe(true)
        expect(document.documentElement.dataset['palette']).toBe('graphite')
        expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull()
    })

    it('follows a later system change while the choice is system', async () => {
        const query = match_media_install(false)
        const theme = await theme_import()

        expect(theme.theme_effective.value).toBe('dark')

        query.matches = true

        for (const listener of change_listeners) listener()

        expect(document.documentElement.classList.contains('theme-light')).toBe(true)
    })

    it('ignores a system change once an explicit theme is set', async () => {
        const query = match_media_install(false)
        const theme = await theme_import()

        theme.theme_set('dark')
        await flush()
        query.matches = true

        for (const listener of change_listeners) listener()

        expect(document.documentElement.classList.contains('theme-dark')).toBe(true)
        expect(document.documentElement.classList.contains('theme-light')).toBe(false)
    })
})

describe('theme_set', () => {
    it('persists an explicit choice to the backend, the html and local storage', async () => {
        match_media_install(true)

        const theme = await theme_import()

        theme.theme_set('dark')
        await flush()

        expect(theme.ipc.settings_update).toHaveBeenCalledWith(
            expect.objectContaining({ theme: 'dark' }),
        )

        expect(document.documentElement.classList.contains('theme-dark')).toBe(true)
        expect(document.documentElement.style.colorScheme).toBe('dark')
        expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark')
    })

    it('stores system as null and clears local storage', async () => {
        match_media_install(true)

        const theme = await theme_import()

        theme.theme_set('dark')
        await flush()
        theme.theme_set('system')
        await flush()

        expect(theme.ipc.settings_update).toHaveBeenLastCalledWith(
            expect.objectContaining({ theme: null }),
        )

        expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull()
        expect(document.documentElement.classList.contains('theme-light')).toBe(true)
    })
})

describe('theme_toggle', () => {
    it('flips between light and dark from the effective theme', async () => {
        match_media_install(true)

        const theme = await theme_import()

        theme.theme_toggle()
        await flush()

        expect(theme.settings.value.theme).toBe('dark')
        expect(theme.theme_effective.value).toBe('dark')

        theme.theme_toggle()
        await flush()

        expect(theme.settings.value.theme).toBe('light')
        expect(document.documentElement.classList.contains('theme-light')).toBe(true)
    })
})

describe('palette_set', () => {
    it('persists the palette and writes it to the html', async () => {
        match_media_install(false)

        const theme = await theme_import()

        theme.palette_set('nord')
        await flush()

        expect(theme.ipc.settings_update).toHaveBeenCalledWith(
            expect.objectContaining({ palette: 'nord' }),
        )

        expect(document.documentElement.dataset['palette']).toBe('nord')
        expect(theme.palette_effective.value).toBe('nord')
    })
})
