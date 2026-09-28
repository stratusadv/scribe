import { computed, ref, shallowRef } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { check } from '@tauri-apps/plugin-updater'
import type { DownloadEvent, Update } from '@tauri-apps/plugin-updater'
import { error_text_extract } from '../lib/errors'
import { ipc } from '../lib/ipc'
import { use_dialog } from './use_dialog'
import { use_tasks } from './use_tasks'


export type UpdatePhase =
    | 'idle'

    | 'checking'
    | 'none'
    | 'available'
    | 'downloading'
    | 'installing'
    | 'error'

const CHECK_TIMEOUT_MS = 15_000
const DOWNLOAD_TIMEOUT_MS = 600_000
const PERCENT_MAX = 100
const phase = ref<UpdatePhase>('idle')
const app_version = ref<string | null>(null)
const update_version = ref<string | null>(null)
const download_bytes_received = ref(0)
const download_bytes_total = ref<number | null>(null)
const error_message = ref<string | null>(null)
const update_pending = shallowRef<Update | null>(null)

const progress_percent = computed(() => {
    const total = download_bytes_total.value

    if (total === null) return null
    if (total === 0) return null

    const fraction = download_bytes_received.value / total

    return Math.min(PERCENT_MAX, Math.round(fraction * PERCENT_MAX))
})

const status_text = computed(() => {
    switch (phase.value) {
        case 'idle': return ''
        case 'checking': return 'Checking for updates…'
        case 'none': return 'scribe is up to date.'
        case 'available': return `scribe ${update_version.value ?? ''} is ready to install.`
        case 'downloading': return status_text_downloading()
        case 'installing': return 'Installing the update. scribe will restart in a moment.'
        case 'error': return error_message.value ?? 'The update check failed.'
    }
})


function status_text_downloading(): string {
    const percent = progress_percent.value

    if (percent === null) return 'Downloading the update…'

    return `Downloading the update… ${percent}%`
}

async function update_release() {
    if (update_pending.value === null) return

    try {
        await update_pending.value.close()
    } catch (error) {
        console.warn('[update] release failed', error)
    } finally {
        update_pending.value = null
    }
}

async function app_version_refresh() {
    if (app_version.value !== null) return

    try {
        app_version.value = await getVersion()
    } catch (error) {
        console.warn('[update] app version unavailable', error)
    }
}

async function update_check(): Promise<boolean> {
    if (phase.value === 'downloading') return false
    if (phase.value === 'installing') return false

    await update_release()

    phase.value = 'checking'
    error_message.value = null

    try {
        const update = await check({ timeout: CHECK_TIMEOUT_MS })

        if (!update) {
            phase.value = 'none'

            return false
        }

        update_pending.value = update
        update_version.value = update.version
        phase.value = 'available'

        return true
    } catch (error) {
        phase.value = 'error'

        error_message.value = 'The update check failed. Check your internet connection '
            + `and try again. (${error_text_extract(error)})`

        return false
    }
}

function download_event_apply(event: DownloadEvent) {
    switch (event.event) {
        case 'Started':
            download_bytes_total.value = event.data.contentLength ?? null

            return
        case 'Progress':
            download_bytes_received.value += event.data.chunkLength

            return
        case 'Finished':
            phase.value = 'installing'
    }
}

async function update_install() {
    const { message: dialog_message } = use_dialog()
    const { tasks_running } = use_tasks()
    const update = update_pending.value

    if (update === null) return

    if (tasks_running.value.length > 0) {
        await dialog_message(
            'scribe is still working on a recording. Wait for it to finish, then install '
                + 'the update from Settings.',
            { title: 'Update waiting', kind: 'info' },
        )

        return
    }

    phase.value = 'downloading'
    download_bytes_received.value = 0
    download_bytes_total.value = null

    try {
        await update.downloadAndInstall(download_event_apply, {
            timeout: DOWNLOAD_TIMEOUT_MS,
        })

        await ipc.app_relaunch()
    } catch (error) {
        phase.value = 'error'

        error_message.value = 'The update could not be installed. scribe keeps working as '
            + `it is; try again later from Settings. (${error_text_extract(error)})`

        await update_release()
        await dialog_message(error_message.value, { title: 'Update failed', kind: 'error' })
    }
}

async function update_prompt(): Promise<void> {
    const { confirm: dialog_confirm } = use_dialog()

    const install_now = await dialog_confirm(
        `A new version of scribe (${update_version.value ?? ''}) is ready. Installing takes `
            + 'about a minute and scribe will restart. Install it now?',
        {
            title: 'Update available',
            kind: 'info',
            ok_label: 'Install now',
            cancel_label: 'Later',
        },
    )

    if (install_now) await update_install()
}

async function update_check_startup() {
    await app_version_refresh()

    if (!navigator.onLine) return

    const available = await update_check()

    if (available) {
        await update_prompt()

        return
    }

    if (phase.value === 'error') {
        console.warn('[scribe] update check failed:', error_message.value)
        phase.value = 'idle'
    }
}

async function update_check_manual() {
    await app_version_refresh()
    await update_check()
}

export function use_update() {
    return {
        phase,
        app_version,
        update_version,
        progress_percent,
        status_text,
        update_check_manual,
        update_check_startup,
        update_install,
    }
}
