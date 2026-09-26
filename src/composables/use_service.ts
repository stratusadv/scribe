import { computed, ref } from 'vue'
import { ipc } from '../lib/ipc'
import type { APIEndpointPurpose, ServiceStatus } from '../types'


const status = ref<ServiceStatus>({
    host: { builtin: false, user: false },
    notes: { builtin: false, user: false },
    transcription: { builtin: false, user: false },
})

const error_message = ref<string | null>(null)
const host_ready = computed(() => status.value.host.builtin || status.value.host.user)

const notes_ready = computed(
    () => host_ready.value && (status.value.notes.builtin || status.value.notes.user),
)

const transcription_ready = computed(
    () => host_ready.value
        && (status.value.transcription.builtin || status.value.transcription.user),
)


async function refresh() {
    error_message.value = null

    try {
        status.value = await ipc.service_status()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function api_key_set(purpose: APIEndpointPurpose, api_key: string) {
    error_message.value = null

    await ipc.service_api_key_set(purpose, api_key)
    await refresh()
}

export function use_service() {
    return {
        status,
        error_message,
        host_ready,
        notes_ready,
        transcription_ready,
        refresh,
        api_key_set,
    }
}
