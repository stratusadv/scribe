import { ref } from 'vue'
import { ipc } from '../lib/ipc'
import type { Settings } from '../types'


const settings_default = Object.freeze<Settings>({
    notes_template_id_default: null,
    theme: null,
    palette: null,
    recordings_directory: null,
    notes_model: null,
    notes_thinking: null,
    api_host: null,
    jobs_view: null,
    speakers: null,
})

const settings = ref<Settings>({ ...settings_default })
const error_message = ref<string | null>(null)


async function refresh() {
    error_message.value = null

    try {
        settings.value = await ipc.settings_get()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function update(patch: Partial<Settings>) {
    error_message.value = null

    try {
        const next: Settings = { ...settings.value, ...patch }

        await ipc.settings_update(next)
        settings.value = next
    } catch (error) {
        error_message.value = String(error)
    }
}

export function use_settings() {
    return { settings, error_message, refresh, update }
}
