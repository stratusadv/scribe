import { nextTick, ref, watch } from 'vue'
import { ipc } from '../lib/ipc'
import { performance_mark, performance_measure } from '../lib/performance'
import { use_notes_templates } from './use_notes_templates'
import { use_settings } from './use_settings'
import type { JobMeta, LineCorrection, Transcript, Waveform } from '../types'


export type View =
    | 'jobs'

    | 'transcribe'
    | 'import'
    | 'details'
    | 'notes'
    | 'settings'
    | 'templates'
    | 'people'

const WAVEFORM_BAR_COUNT_DEFAULT = 360
const view_current = ref<View>('jobs')
const job_id_current = ref<string | null>(null)
const source_path = ref<string | null>(null)
const transcript = ref<Transcript | null>(null)
const notes_markdown = ref<string>('')
const template_id_selected = ref<string | null>(null)
const meta_current = ref<JobMeta | null>(null)
const waveform_current = ref<Waveform | null>(null)
const job_open_busy = ref(false)
const view_reopen_count = ref(0)


watch(view_current, () => {
    view_reopen_count.value = 0
})

function view_paint_measure() {
    requestAnimationFrame(() => {
        requestAnimationFrame(() => {
            performance_measure('view_switch', 'view_switch_start')
        })
    })
}

function view_set(view: View) {
    if (view_current.value === view) return

    performance_mark('view_switch_start')
    view_current.value = view

    void nextTick(view_paint_measure)
}

function view_reopen(view: View) {
    if (view_current.value === view) {
        view_reopen_count.value += 1

        return
    }

    view_set(view)
}

function pipeline_reset() {
    job_id_current.value = null
    source_path.value = null
    transcript.value = null
    notes_markdown.value = ''
    template_id_selected.value = null
    meta_current.value = null
    waveform_current.value = null
    view_current.value = 'jobs'
}

function template_id_ensure() {
    const { templates } = use_notes_templates()
    const { settings } = use_settings()
    const candidate = template_id_selected.value ?? settings.value.notes_template_id_default
    const known = templates.value.some((template) => template.id === candidate)

    template_id_selected.value = known ? candidate : null
}

async function job_open(meta: JobMeta, view_target: View | null) {
    if (job_open_busy.value) return

    job_open_busy.value = true

    try {
        await job_open_commit(meta, view_target)
    } finally {
        job_open_busy.value = false
    }
}

async function job_open_commit(meta: JobMeta, view_target: View | null) {
    const engines = await ipc.job_transcript_engines_list(meta.id)
    const [engine_first] = engines

    const [transcript_loaded, notes_loaded] = await Promise.all([
        engine_first ? ipc.job_transcript_load(meta.id, engine_first) : null,
        ipc.notes_load(meta.id),
    ])

    job_id_current.value = meta.id
    source_path.value = meta.source_path
    meta_current.value = meta
    transcript.value = transcript_loaded
    notes_markdown.value = notes_loaded ?? ''
    waveform_current.value = null

    void waveform_refresh()

    view_current.value = view_target ?? view_for_job_state()
}

function view_for_job_state(): View {
    if (notes_markdown.value) return 'notes'
    if (transcript.value) return 'details'

    return 'transcribe'
}

async function waveform_refresh() {
    const id = job_id_current.value

    if (!id) {
        waveform_current.value = null

        return
    }

    try {
        waveform_current.value = await ipc.job_waveform_get(id, WAVEFORM_BAR_COUNT_DEFAULT)
    } catch (error) {
        waveform_current.value = null

        console.warn('[pipeline] waveform unavailable', id, error)
    }
}

async function meta_refresh() {
    const id = job_id_current.value

    if (!id) return

    try {
        const fresh = await ipc.job_meta_get(id)

        if (fresh) meta_current.value = fresh
    } catch (error) {
        console.warn('[pipeline] meta refresh failed', id, error)
    }
}

async function transcript_segments_text_set(corrections: LineCorrection[]) {
    const current = transcript.value

    if (!current) return
    if (corrections.length === 0) return

    const text_by_index = new Map(
        corrections.map((correction) => [correction.index, correction.text]),
    )

    const segments = current.segments.map((segment, index) => {
        const text = text_by_index.get(index)

        return text === undefined ? segment : { ...segment, text }
    })

    const text_full = segments
        .map((segment) => segment.text.trim())
        .filter((segment_text) => segment_text.length > 0)
        .join(' ')

    transcript.value = { text: text_full, segments }

    const id = job_id_current.value

    if (id) await ipc.job_transcript_save(id, transcript.value)
}

function job_start_new() {
    pipeline_reset()
    view_current.value = 'transcribe'
}

export function use_pipeline() {
    return {
        view_current,
        job_id_current,
        source_path,
        transcript,
        notes_markdown,
        template_id_selected,
        meta_current,
        waveform_current,
        job_open_busy,
        view_reopen,
        view_reopen_count,
        view_set,
        pipeline_reset,
        job_open,
        job_start_new,
        meta_refresh,
        template_id_ensure,
        transcript_segments_text_set,
        waveform_refresh,
    }
}
