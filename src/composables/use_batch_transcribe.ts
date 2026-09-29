import { computed, ref } from 'vue'
import { ipc } from '../lib/ipc'
import { path_file_name_short } from '../lib/paths'
import { use_endpoints } from './use_endpoints'
import { use_service } from './use_service'
import { use_tasks } from './use_tasks'


type QueueStatus = 'pending' | 'running' | 'succeeded' | 'failed'

interface QueueEntry {
    path: string
    status: QueueStatus
    error_message: string | null
    job_id: string | null
}

const QUEUE_LENGTH_MAX = 256
const MESSAGE_KEY_MISSING = 'Add your transcription key under Settings, then drop the files again.'
const MESSAGE_QUEUE_FULL = `Drop at most ${QUEUE_LENGTH_MAX} files at a time.`
const queue = ref<QueueEntry[]>([])
const running = ref(false)
const error_message_last = ref<string | null>(null)

const pending_count = computed(
    () => queue.value.filter((entry) => entry.status === 'pending').length,
)

const running_count = computed(
    () => queue.value.filter((entry) => entry.status === 'running').length,
)


function entry_replace(path: string, patch: Partial<QueueEntry>) {
    queue.value = queue.value.map((entry) =>
        entry.path === path ? { ...entry, ...patch } : entry,
    )
}

function entry_pending_next(): QueueEntry | undefined {
    return queue.value.find((entry) => entry.status === 'pending')
}

function transcription_configured(): boolean {
    const { endpoints_by_purpose } = use_endpoints()
    const { transcription_ready } = use_service()

    if (!transcription_ready.value) return false

    return endpoints_by_purpose('transcription').length > 0
}

async function enqueue(paths: string[]) {
    error_message_last.value = null

    if (!transcription_configured()) {
        error_message_last.value = MESSAGE_KEY_MISSING

        return
    }

    if (queue.value.length + paths.length > QUEUE_LENGTH_MAX) {
        error_message_last.value = MESSAGE_QUEUE_FULL

        return
    }

    const entries_new: QueueEntry[] = paths.map((path) => ({
        path,
        status: 'pending',
        error_message: null,
        job_id: null,
    }))

    queue.value = [...queue.value, ...entries_new]

    if (running.value) return

    running.value = true

    try {
        await queue_drain()
    } finally {
        running.value = false
    }
}

async function queue_drain() {
    let next = entry_pending_next()

    while (next) {
        await transcribe_one(next.path)

        next = entry_pending_next()
    }
}

async function transcribe_one(path: string) {
    const { endpoints_by_purpose } = use_endpoints()
    const { task_run } = use_tasks()

    entry_replace(path, { status: 'running' })

    try {
        const endpoint_id = endpoints_by_purpose('transcription')[0]?.id ?? null

        await task_run({
            kind: 'transcribe',
            label: path_file_name_short(path),
            async runner() {
                if (!endpoint_id) throw new Error(MESSAGE_KEY_MISSING)

                return await ipc.transcription_remote(path, endpoint_id, null, true)
            },
            on_success(result, task) {
                task.job_id = result.job_id

                entry_replace(path, { status: 'succeeded', job_id: result.job_id })
            },
        })
    } catch (error) {
        entry_replace(path, { status: 'failed', error_message: String(error) })
    }
}

function entries_clear_finished() {
    queue.value = queue.value.filter(
        (entry) => entry.status !== 'succeeded' && entry.status !== 'failed',
    )
}

export function use_batch_transcribe() {
    return {
        queue,
        running,
        pending_count,
        running_count,
        error_message_last,
        enqueue,
        entries_clear_finished,
    }
}
