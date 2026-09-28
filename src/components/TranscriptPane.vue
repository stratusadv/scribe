<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { convertFileSrc } from '@tauri-apps/api/core'
import { ipc } from '../lib/ipc'
import { error_dialog_show } from '../lib/errors'
import { seconds_to_clock } from '../lib/duration'
import { use_pipeline } from '../composables/use_pipeline'
import { use_shortcuts } from '../composables/use_shortcuts'
import type { ComponentPublicInstance } from 'vue'
import type { Transcript, TranscriptSegment } from '../types'


interface DisplayRow {
    text: string
    start_seconds: number
    end_seconds: number
    segment_index: number
}

const ROWS_RENDER_STEP = 60
const ROWS_RENDER_THRESHOLD_PX = 200
const COPIED_FLASH_MS = 1500
const BLOB_PREFIX = 'blob:'
const HIGHLIGHT_NAME = 'transcript-filter'
const REGEX_ESCAPE = /[.*+?^${}()|[\]\\]/g
const { job_id_current, transcript_segments_text_set } = use_pipeline()

const props = defineProps<{
    transcript: Transcript | null
    segment_indexes_highlighted?: number[]
}>()

const filter_text = ref('')
const index_copied = ref<number | null>(null)
const index_editing = ref<number | null>(null)
const text_editing = ref('')
const list_ref = ref<HTMLElement | null>(null)
const audio_ref = ref<HTMLAudioElement | null>(null)
const audio_src = ref<string | null>(null)
const audio_checked = ref(false)
const playback_index = ref<number | null>(null)
const playback_end_seconds = ref<number | null>(null)
const rows_render_limit = ref(ROWS_RENDER_STEP)

const rows = computed<DisplayRow[]>(() => {
    if (!props.transcript) return []

    return props.transcript.segments
        .map((segment: TranscriptSegment, segment_index: number) => ({
            text: segment.text.trim(),
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
            segment_index,
        }))
        .filter((row) => row.text.length > 0)
})

const filter_regex = computed<RegExp | null>(() => {
    const needle = filter_text.value.trim()

    if (needle.length === 0) return null

    return new RegExp(needle.replace(REGEX_ESCAPE, '\\$&'), 'i')
})

const rows_filtered = computed<DisplayRow[]>(() => {
    const regex = filter_regex.value

    if (!regex) return rows.value

    return rows.value.filter((row) => regex.test(row.text))
})

const rows_rendered = computed<DisplayRow[]>(
    () => rows_filtered.value.slice(0, rows_render_limit.value),
)


watch(rows_filtered, () => {
    rows_render_limit.value = ROWS_RENDER_STEP
    index_editing.value = null
})

watch(rows_rendered, async () => {
    await nextTick()

    rows_render_grow()
})

watch([rows_rendered, filter_regex, index_editing], highlights_apply, { flush: 'post' })

function highlights_registry(): HighlightRegistry | null {
    if (typeof CSS === 'undefined') return null

    return CSS.highlights
}

function highlights_apply() {
    const registry = highlights_registry()
    const regex = filter_regex.value
    const container = list_ref.value

    if (!registry) return

    if (!regex || !container) {
        registry.delete(HIGHLIGHT_NAME)

        return
    }

    const matcher = new RegExp(regex.source, 'gi')
    const ranges: Range[] = []

    for (const element of container.querySelectorAll('.transcript-pane-row-text')) {
        const node = element.firstChild

        if (!(node instanceof Text)) continue

        for (const match of node.data.matchAll(matcher)) {
            const range = new Range()

            range.setStart(node, match.index)
            range.setEnd(node, match.index + match[0].length)
            ranges.push(range)
        }
    }

    registry.set(HIGHLIGHT_NAME, new Highlight(...ranges))
}

function rows_render_grow() {
    const container = list_ref.value

    if (!container) return
    if (rows_render_limit.value >= rows_filtered.value.length) return

    const distance_to_end = container.scrollHeight - container.scrollTop - container.clientHeight

    if (distance_to_end > ROWS_RENDER_THRESHOLD_PX) return

    rows_render_limit.value += ROWS_RENDER_STEP
}

async function row_copy(row: DisplayRow, index: number) {
    try {
        await navigator.clipboard.writeText(row.text)
        index_copied.value = index

        setTimeout(() => {
            if (index_copied.value === index) index_copied.value = null
        }, COPIED_FLASH_MS)
    } catch (error) {
        index_copied.value = null

        console.warn('[transcript] clipboard write failed', error)
    }
}

function row_highlighted(row: DisplayRow): boolean {
    return props.segment_indexes_highlighted?.includes(row.segment_index) ?? false
}

function row_edit_toggle(row: DisplayRow, index: number, event: Event) {
    event.stopPropagation()

    if (index_editing.value === index) {
        void row_edit_commit(row, index)

        return
    }

    index_editing.value = index
    text_editing.value = row.text
}

function row_edit_focus(element: Element | ComponentPublicInstance | null) {
    if (element instanceof HTMLTextAreaElement) element.focus()
}

function row_edit_cancel() {
    index_editing.value = null
}

async function row_edit_commit(row: DisplayRow, index: number) {
    if (index_editing.value !== index) return

    const text = text_editing.value.trim()

    index_editing.value = null

    if (text.length === 0) return
    if (text === row.text) return

    try {
        await transcript_segments_text_set([{ index: row.segment_index, text }])
    } catch (error) {
        await error_dialog_show(error)
    }
}

function audio_src_release() {
    if (audio_src.value?.startsWith(BLOB_PREFIX)) URL.revokeObjectURL(audio_src.value)

    audio_src.value = null
}

async function audio_src_load() {
    audio_src_release()

    const id = job_id_current.value

    if (!id) {
        audio_checked.value = true

        return
    }

    try {
        const path = await ipc.job_audio_path_get(id)

        if (path) audio_src.value = convertFileSrc(path)
    } catch (error) {
        audio_src.value = null

        console.warn('[transcript] audio path unavailable', id, error)
    } finally {
        audio_checked.value = true
    }
}

async function audio_error_fallback() {
    const source = audio_src.value

    if (!source || source.startsWith(BLOB_PREFIX)) {
        audio_src_release()

        return
    }

    try {
        const response = await fetch(source)

        if (!response.ok) throw new Error(`audio fetch: ${response.status}`)

        audio_src.value = URL.createObjectURL(await response.blob())
    } catch (error) {
        audio_src_release()

        console.warn('[transcript] audio fallback failed', error)
    }
}

function playback_clear() {
    playback_index.value = null
    playback_end_seconds.value = null
}

function audio_time_update() {
    const audio = audio_ref.value

    if (!audio) return
    if (playback_end_seconds.value === null) return
    if (audio.currentTime < playback_end_seconds.value) return

    audio.pause()
    playback_clear()
}

function audio_pause_handler() {
    if (audio_ref.value?.paused) playback_clear()
}

async function row_play_toggle(row: DisplayRow, index: number, event: Event) {
    event.stopPropagation()

    const audio = audio_ref.value

    if (!audio || !audio_src.value) return

    if (playback_index.value === index) {
        audio.pause()
        playback_clear()

        return
    }

    audio.pause()
    audio.currentTime = Math.max(0, row.start_seconds)
    playback_end_seconds.value = row.end_seconds
    playback_index.value = index

    try {
        await audio.play()
    } catch (error) {
        playback_clear()

        console.warn('[transcript] playback refused', error)
    }
}

function playback_space_toggle() {
    const audio = audio_ref.value

    if (!audio || !audio_src.value) return

    if (audio.paused) {
        void audio.play().catch((error: unknown) => {
            console.warn('[transcript] playback refused', error)
        })

        return
    }

    audio.pause()
}

function audio_pause_if_playing() {
    if (audio_ref.value && !audio_ref.value.paused) audio_ref.value.pause()
}

use_shortcuts({ space: playback_space_toggle })

onMounted(() => {
    void audio_src_load()
})

watch(() => job_id_current.value, () => {
    audio_pause_if_playing()
    playback_clear()

    void audio_src_load()
})

onUnmounted(() => {
    audio_pause_if_playing()
    audio_src_release()
    highlights_registry()?.delete(HIGHLIGHT_NAME)
})
</script>

<template>
    <div class="transcript-pane card">
        <audio
            v-if="audio_src"
            ref="audio_ref"
            :src="audio_src"
            preload="metadata"
            @timeupdate="audio_time_update"
            @pause="audio_pause_handler"
            @error="audio_error_fallback"
        />
        <div class="transcript-pane-header">
            <div class="text-sm font-medium">Transcript</div>
            <input
                v-model="filter_text"
                type="text"
                class="input transcript-pane-filter"
                placeholder="Filter…"
            />
        </div>

        <div v-if="rows.length === 0" class="meta px-3 py-2">
            There is no transcript yet.
        </div>
        <ul
            v-else-if="audio_checked"
            ref="list_ref"
            class="transcript-pane-list"
            @scroll.passive="rows_render_grow"
        >
            <li
                v-for="(row, index) in rows_rendered"
                :key="index"
                class="transcript-pane-row"
                :data-row-index="index"
                :data-copied="index_copied === index ? 'true' : 'false'"
                :data-playing="playback_index === index ? 'true' : 'false'"
                :data-highlighted="row_highlighted(row) ? 'true' : 'false'"
                :data-editing="index_editing === index ? 'true' : 'false'"
                :title="'Click to copy'"
                @click="row_copy(row, index)"
            >
                <div class="transcript-pane-actions">
                    <button
                        v-if="audio_src"
                        type="button"
                        class="transcript-pane-play"
                        :title="playback_index === index ? 'Pause' : 'Play segment'"
                        @click="row_play_toggle(row, index, $event)"
                    >
                        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                            <template v-if="playback_index === index">
                                <rect x="6" y="5" width="4" height="14" rx="1" />
                                <rect x="14" y="5" width="4" height="14" rx="1" />
                            </template>
                            <path v-else d="M8 5.5v13a1 1 0 0 0 1.5.87l11-6.5a1 1 0 0 0 0-1.74l-11-6.5A1 1 0 0 0 8 5.5Z" />
                        </svg>
                    </button>
                    <button
                        type="button"
                        class="transcript-pane-edit"
                        :title="index_editing === index ? 'Done' : 'Correct this line'"
                        @mousedown.prevent
                        @click="row_edit_toggle(row, index, $event)"
                    >
                        <svg
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="3"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            aria-hidden="true"
                        >
                            <path d="M4 20h4l11-11-4-4L4 16v4Z" />
                        </svg>
                    </button>
                </div>
                <div class="transcript-pane-row-body">
                    <div class="transcript-pane-row-meta">
                        <span class="transcript-pane-row-time">
                            {{ seconds_to_clock(row.start_seconds) }}
                        </span>
                        <span v-if="index_copied === index" class="pill-active">Copied</span>
                    </div>
                    <textarea
                        v-if="index_editing === index"
                        :ref="row_edit_focus"
                        v-model="text_editing"
                        class="transcript-pane-row-text transcript-pane-row-edit"
                        rows="1"
                        aria-label="Transcript line"
                        @click.stop
                        @keydown.enter.prevent="row_edit_commit(row, index)"
                        @keydown.esc="row_edit_cancel"
                        @blur="row_edit_commit(row, index)"
                    />
                    <div v-else class="transcript-pane-row-text">{{ row.text }}</div>
                </div>
            </li>
            <li v-if="rows_filtered.length === 0" class="meta px-3 py-2">
                No segments match.
            </li>
        </ul>
        <p class="transcript-pane-footer">
            Click a line to copy it. Use the pencil to correct it.
        </p>
    </div>
</template>
