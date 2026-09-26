import { computed, ref } from 'vue'
import {
    isPermissionGranted,
    requestPermission,
    sendNotification,
} from '@tauri-apps/plugin-notification'
import { unix_now } from '../lib/duration'
import { ipc } from '../lib/ipc'


export type TaskKind = 'transcribe' | 'generate'
export type TaskStatus = 'running' | 'succeeded' | 'failed' | 'cancelled'
type PermissionState = 'unknown' | 'granted' | 'denied'

export interface Task {
    id: string
    kind: TaskKind
    label: string
    started_at_unix: number
    finished_at_unix: number | null
    status: TaskStatus
    error_message: string | null
    job_id: string | null
    stream_id: string | null
}

interface TaskRunOptions<T> {
    kind: TaskKind
    label: string
    runner: () => Promise<T>
    on_success?: (result: T, task: Task) => void | Promise<void>
    stream_id?: string | null
}

const TASK_RETENTION_MAX = 8
const CANCELLED_PATTERN = /cancelled/i
const tasks_all = ref<Task[]>([])
const tasks = computed(() => tasks_all.value)
const tasks_running = computed(() => tasks_all.value.filter((task) => task.status === 'running'))
const tasks_finished = computed(() => tasks_all.value.filter((task) => task.status !== 'running'))
const permission_state = ref<PermissionState>('unknown')


export function is_cancelled_error(error: unknown): boolean {
    const message = error instanceof Error ? error.message : String(error)

    return CANCELLED_PATTERN.test(message)
}

function task_replace(id: string, patch: Partial<Task>) {
    tasks_all.value = tasks_all.value.map((task) => (task.id === id ? { ...task, ...patch } : task))
}

function task_find(id: string): Task | undefined {
    return tasks_all.value.find((task) => task.id === id)
}

function task_prune() {
    if (tasks_all.value.length <= TASK_RETENTION_MAX) return

    tasks_all.value = tasks_all.value.slice(-TASK_RETENTION_MAX)
}

async function notification_permission_ensure(): Promise<boolean> {
    if (permission_state.value === 'granted') return true
    if (permission_state.value === 'denied') return false

    try {
        const granted_already = await isPermissionGranted()

        if (granted_already) {
            permission_state.value = 'granted'

            return true
        }

        const result = await requestPermission()

        permission_state.value = result === 'granted' ? 'granted' : 'denied'

        return permission_state.value === 'granted'
    } catch {
        permission_state.value = 'denied'

        return false
    }
}

function window_is_visible(): boolean {
    if (typeof document === 'undefined') return true

    return document.visibilityState === 'visible' && document.hasFocus()
}

function task_notification_title(task: Task): string {
    if (task.status === 'failed') {
        switch (task.kind) {
            case 'transcribe': return 'Transcription failed'
            case 'generate': return 'Notes generation failed'
        }
    }

    if (task.status === 'cancelled') return 'Cancelled'

    switch (task.kind) {
        case 'transcribe': return 'Transcription finished'
        case 'generate': return 'Notes ready'
    }
}

async function notification_send_if_appropriate(task: Task) {
    if (task.status === 'running') return

    if (task.status === 'succeeded') {
        if (window_is_visible()) return
    }

    const granted = await notification_permission_ensure()

    if (!granted) return

    try {
        const title = task_notification_title(task)

        const body = task.status === 'failed' && task.error_message
            ? task.error_message
            : task.label

        sendNotification({ title, body })
    } catch (error) {
        console.warn('[tasks] notification not sent', error)
    }
}

async function task_run<T>(options: TaskRunOptions<T>): Promise<T> {
    const id = crypto.randomUUID()

    const task_initial: Task = {
        id,
        kind: options.kind,
        label: options.label,
        started_at_unix: unix_now(),
        finished_at_unix: null,
        status: 'running',
        error_message: null,
        job_id: null,
        stream_id: options.stream_id ?? null,
    }

    tasks_all.value = [...tasks_all.value, task_initial]

    task_prune()

    try {
        const result = await options.runner()

        await task_finish_success(id, result, options.on_success)

        return result
    } catch (error) {
        await task_finish_failure(id, error)

        throw error
    }
}

async function task_finish_success<T>(
    id: string,
    result: T,
    on_success: TaskRunOptions<T>['on_success'],
) {
    task_replace(id, { status: 'succeeded', finished_at_unix: unix_now() })

    if (on_success) {
        const current = task_find(id)

        if (current) await on_success(result, current)
    }

    const task_final = task_find(id)

    if (task_final) await notification_send_if_appropriate(task_final)
}

async function task_finish_failure(id: string, error: unknown) {
    const cancelled = is_cancelled_error(error)

    task_replace(id, {
        status: cancelled ? 'cancelled' : 'failed',
        finished_at_unix: unix_now(),
        error_message: cancelled ? null : String(error),
    })

    if (cancelled) return

    const task_final = task_find(id)

    if (task_final) await notification_send_if_appropriate(task_final)
}

function task_dismiss(id: string) {
    tasks_all.value = tasks_all.value.filter((task) => task.id !== id)
}

async function task_cancel(id: string) {
    const task = task_find(id)

    if (!task) return
    if (task.status !== 'running') return
    if (!task.stream_id) return

    try {
        await ipc.task_cancel(task.stream_id)
    } catch (error) {
        console.warn('[tasks] cancel not delivered', error)
    }
}

function tasks_clear_finished() {
    tasks_all.value = tasks_all.value.filter((task) => task.status === 'running')
}

export function use_tasks() {
    return {
        tasks,
        tasks_running,
        tasks_finished,
        task_run,
        task_dismiss,
        task_cancel,
        tasks_clear_finished,
    }
}
