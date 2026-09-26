import { ref } from 'vue'
import { ipc } from '../lib/ipc'
import type { JobListing, Waveform } from '../types'


const WAVEFORM_BAR_COUNT_TILE = 110
const jobs = ref<JobListing[]>([])
const error_message = ref<string | null>(null)
const waveforms = ref<Map<string, Waveform | null>>(new Map())


async function refresh() {
    error_message.value = null

    try {
        jobs.value = await ipc.jobs_list()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function favourite_set(job_id: string, favourite: boolean) {
    error_message.value = null

    try {
        await ipc.job_meta_update(job_id, { favourite })
        await refresh()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function remove(job_id: string) {
    error_message.value = null

    try {
        await ipc.job_delete(job_id)
        waveforms.value.delete(job_id)
        await refresh()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function waveform_load(job_id: string): Promise<Waveform | null> {
    const cached = waveforms.value.get(job_id)

    if (cached !== undefined) return cached

    const waveform = await ipc.job_waveform_get(job_id, WAVEFORM_BAR_COUNT_TILE)

    waveforms.value.set(job_id, waveform)

    return waveform
}

export function use_jobs() {
    return { error_message, favourite_set, jobs, refresh, remove, waveform_load }
}
