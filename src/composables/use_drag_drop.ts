import { onMounted, onUnmounted, ref, shallowRef } from 'vue'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import type { Event, UnlistenFn } from '@tauri-apps/api/event'
import type { DragDropEvent } from '@tauri-apps/api/webviewWindow'
import { path_is_media } from '../lib/media'
import { use_batch_transcribe } from './use_batch_transcribe'
import { use_pipeline } from './use_pipeline'


const dragging_over = ref(false)
const listener = shallowRef<UnlistenFn | null>(null)
const registration_count = ref(0)


function paths_dropped(paths: string[]) {
    const { source_path, transcript, job_id_current, view_set } = use_pipeline()
    const { enqueue } = use_batch_transcribe()
    const media_paths = paths.filter(path_is_media)

    if (media_paths.length === 0) return

    if (media_paths.length === 1) {
        source_path.value = media_paths[0] ?? null
        transcript.value = null
        job_id_current.value = null

        view_set('transcribe')

        return
    }

    void enqueue(media_paths)
}

function drag_drop_event_apply(event: Event<DragDropEvent>) {
    switch (event.payload.type) {
        case 'enter':
        case 'over':
            dragging_over.value = true

            return
        case 'leave':
            dragging_over.value = false

            return
        case 'drop':
            dragging_over.value = false

            paths_dropped(event.payload.paths)
    }
}

async function listener_register() {
    if (listener.value !== null) {
        registration_count.value += 1

        return
    }

    const window = getCurrentWebviewWindow()

    listener.value = await window.onDragDropEvent(drag_drop_event_apply)
    registration_count.value += 1
}

function listener_release() {
    registration_count.value -= 1

    if (registration_count.value > 0) return

    registration_count.value = 0

    if (listener.value === null) return

    listener.value()
    listener.value = null
}

export function use_drag_drop() {
    onMounted(listener_register)
    onUnmounted(listener_release)

    return { dragging_over }
}
