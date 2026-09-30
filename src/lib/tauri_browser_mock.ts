import { assert } from './assert'
import type {
    APIEndpoint,
    APIEndpointPurpose,
    Group,
    JobMeta,
    JobMetaPatch,
    JobSearchHit,
    NotesTemplate,
    Person,
    ServiceStatus,
    Settings,
    Transcript,
    TranscriptionResult,
    Waveform,
} from '../types'


type InvokeArgs = Record<string, unknown>
type CommandHandler = (call: InvokeArgs) => unknown
type ListTag = 'ul' | 'ol'

interface CallbackEntry {
    callback: (payload: unknown) => void
    once: boolean
}

interface ListenerEntry {
    event: string
    callback_id: number
}

interface DialogButtons {
    ok?: string
    cancel?: string
}

interface MockStore {
    settings: Settings
    service_api_keys: Record<APIEndpointPurpose, string>
    endpoints: APIEndpoint[]
    notes_templates: NotesTemplate[]
    people: Person[]
    groups: Group[]
    jobs: JobMeta[]
    notes_markdown: Map<string, string>
    transcripts: Map<string, Transcript>
    callbacks: Map<number, CallbackEntry>
    listeners: Map<number, ListenerEntry>
    streams_cancelled: Set<string>
    callback_id_next: number
    listener_id_next: number
}

const CHUNK_DELAY_MS = 45
const SEGMENT_DELAY_MS = 260
const SECONDS_PER_LINE = 18
const DURATION_SECONDS_SAMPLE = 144
const SOURCE_PATH_IMPORTED = '(imported transcript)'
const WAVEFORM_BAR_COUNT_MAX = 2000
const SNIPPET_CHARS_BEFORE = 40
const SNIPPET_CHARS_AFTER = 80
const ENDPOINT_ID_NOTES = 'builtin-notes'
const ENDPOINT_ID_TRANSCRIPTION = 'builtin-transcription'
const API_HOST = 'https://ai.example.invalid'
const JOB_ID_SAMPLE = '86dfb1d3b0140ecf2fd59075421f1cb5'
const JOB_ID_SAMPLE_SECOND = '47ead100cc5cd956aa10b3c4d5e6f708'
const RECORDING_PATH_SAMPLE = 'C:\\Users\\mock\\Music\\scribe\\Recording mock.wav'
const RECORDINGS_DIRECTORY_SAMPLE = 'C:\\Users\\mock\\Music\\scribe'
const store = store_new()


function settings_seed(): Settings {
    return {
        notes_template_id_default: 'default-meeting-notes',
        theme: 'dark',
        palette: null,
        recordings_directory: null,
        notes_model: null,
        notes_thinking: null,
        api_host: null,
        jobs_view: null,
    }
}

function endpoints_seed(): APIEndpoint[] {
    return [
        {
            id: ENDPOINT_ID_NOTES,
            name: 'Stratus Thinking (browser mock)',
            purpose: 'notes',
            host: API_HOST,
            api_key: '',
            model: 'stratus.thinking',
            temperature: null,
            output_tokens_max: null,
            api_path_chat: null,
            api_path_transcribe: null,
            transcribe_verbose: null,
            transcribe_chunk_seconds: null,
            has_api_key: true,
        },
        {
            id: ENDPOINT_ID_TRANSCRIPTION,
            name: 'Stratus Listen (browser mock)',
            purpose: 'transcription',
            host: API_HOST,
            api_key: '',
            model: 'stratus.listen',
            temperature: null,
            output_tokens_max: null,
            api_path_chat: null,
            api_path_transcribe: null,
            transcribe_verbose: false,
            transcribe_chunk_seconds: 30,
            has_api_key: true,
        },
    ]
}

function notes_templates_seed(): NotesTemplate[] {
    return [
        {
            id: 'default-meeting-notes',
            name: 'Meeting notes',
            description: 'A summary, the decisions, and who does what next.',
            instructions: [
                '## Summary',
                '',
                'A short summary.',
                '',
                '## Decisions',
                '',
                'What was decided.',
                '',
                '## To do',
                '',
                '- [ ] Who does what.',
            ].join('\n'),
            edited: false,
        },
        {
            id: 'default-standup',
            name: 'Stand-up digest',
            description: 'What each person did, what is next, and what is blocking them.',
            instructions: [
                '## Done',
                '',
                'What each person did.',
                '',
                '## Next',
                '',
                'What they will do next.',
                '',
                '## Blocked',
                '',
                'What is in their way.',
            ].join('\n'),
            edited: false,
        },
    ]
}

function people_seed(): Person[] {
    return [
        {
            id: 'person-john',
            name_first: 'John',
            name_last: 'Doe',
            role: 'Developer',
            description: 'Builds scribe. Usually runs the platform sync.',
        },
        {
            id: 'person-jane',
            name_first: 'Jane',
            name_last: 'Doe',
            role: 'Product owner',
            description: 'Decides what ships next.',
        },
        {
            id: 'person-sam',
            name_first: 'Sam',
            name_last: 'Smith',
            role: 'Client',
            description: '',
        },
    ]
}

function jobs_seed(): JobMeta[] {
    return [
        {
            id: JOB_ID_SAMPLE,
            source_path: '/home/you/recordings/weekly-sync.m4a',
            source_size_bytes: 55326134,
            created_at_unix: 1775000000,
            recorded_at_unix: 1774990000,
            label: 'weekly-sync.m4a',
            title: 'Weekly platform sync',
            attendees: ['John Doe', 'Jane Doe', 'Sam Smith'],
            person_ids: ['person-john', 'person-jane'],
            person_ids_mentioned: ['person-sam'],
            project: 'scribe',
            tags: ['meeting-notes', 'platform'],
            favourite: true,
        },
        {
            id: JOB_ID_SAMPLE_SECOND,
            source_path: '/home/you/recordings/discovery-call.wav',
            source_size_bytes: 18220400,
            created_at_unix: 1774600000,
            recorded_at_unix: null,
            label: 'discovery-call.wav',
            title: null,
            attendees: [],
            person_ids: [],
            person_ids_mentioned: [],
            project: null,
            tags: [],
            favourite: false,
        },
    ]
}

function notes_markdown_seed(): Map<string, string> {
    return new Map<string, string>([
        [
            JOB_ID_SAMPLE,
            [
                '# Weekly platform sync',
                '',
                '## Summary',
                '',
                'The team agreed to ship the transcription retry work this week [0:12].',
                'The offline banner copy is due before the release [1:48].',
                '',
                '## Decisions',
                '',
                '- Retry policy lives in one place per integration [0:34]',
                '- Chunked upload stays capped at eight parallel requests [2:05]',
                '',
                '## Action items',
                '',
                '- [ ] Dana: measure upload latency on a 90 minute recording [2:31]',
                '- [ ] Sam: draft the offline banner copy [3:02]',
            ].join('\n'),
        ],
    ])
}

function transcripts_seed(): Map<string, Transcript> {
    return new Map<string, Transcript>([
        [JOB_ID_SAMPLE, transcript_build([
            'Alright, everyone is here, so let us start with transcription.',
            'The retry work is close. I want it merged before Thursday.',
            'One caveat: the remote endpoint rejects verbose_json, so segments are synthesized.',
            'That means the waveform overlay is coarser than it looks on Whisper proper.',
            'The release stays behind the flag until the copy lands.',
            'Dana, can you measure upload latency on a ninety minute recording?',
            'Yes. I will have numbers by tomorrow afternoon.',
            'Sam is drafting the offline banner copy, so we can close that one out.',
        ])],
        [JOB_ID_SAMPLE_SECOND, transcript_build([
            'Thanks for making time. Tell me about your current note taking.',
            'We record everything and then nobody writes it up.',
            'That is the gap we are trying to close.',
        ])],
    ])
}

function store_new(): MockStore {
    return {
        settings: settings_seed(),
        service_api_keys: { notes: 'mock-key', transcription: 'mock-key' },
        endpoints: endpoints_seed(),
        notes_templates: notes_templates_seed(),
        people: people_seed(),
        groups: [
            { id: 'group-team', name: 'Product team', person_ids: ['person-john', 'person-jane'] },
        ],
        jobs: jobs_seed(),
        notes_markdown: notes_markdown_seed(),
        transcripts: transcripts_seed(),
        callbacks: new Map<number, CallbackEntry>(),
        listeners: new Map<number, ListenerEntry>(),
        streams_cancelled: new Set<string>(),
        callback_id_next: 1,
        listener_id_next: 1,
    }
}

export function tauri_browser_mock_install() {
    const target = window as unknown as Record<string, unknown>

    target['__TAURI_INTERNALS__'] = {
        invoke: invoke_mock,
        'transformCallback': callback_transform,
        'unregisterCallback': callback_unregister,
        'convertFileSrc': file_source_convert,
        metadata: {
            'currentWindow': { label: 'main' },
            'currentWebview': { label: 'main' },
        },
        plugins: {},
    }

    target['__TAURI_EVENT_PLUGIN_INTERNALS__'] = {
        'unregisterListener': listener_unregister,
    }

    console.info('[scribe] browser mock backend active — no Tauri, no real network calls')
}

const commands = Object.freeze<Record<string, CommandHandler>>({
    settings_get() {
        return { ...store.settings }
    },
    settings_update(call) {
        Object.assign(store.settings, call['settings'] as Settings)
    },

    notes_models_list() {
        return {
            models: ['stratus.qwen3827b', 'stratus.thinking', 'stratus.turbo'],
            model_default: 'stratus.thinking',
        }
    },
    endpoints_list() {
        return store.endpoints.map((endpoint) => ({
            ...endpoint,
            has_api_key: store.service_api_keys[endpoint.purpose].length > 0,
        }))
    },
    service_status() {
        const status: ServiceStatus = {
            host: { builtin: true, user: store.settings.api_host !== null },
            notes: { builtin: false, user: store.service_api_keys.notes.length > 0 },
            transcription: {
                builtin: false,
                user: store.service_api_keys.transcription.length > 0,
            },
        }

        return status
    },
    service_api_key_set(call) {
        const purpose = call['purpose'] as APIEndpointPurpose

        store.service_api_keys[purpose] = (call['api_key'] as string).trim()
    },

    notes_templates_list() {
        return store.notes_templates.map((template) => ({ ...template }))
    },
    notes_template_save(call) {
        const template = call['template'] as NotesTemplate
        const index = store.notes_templates.findIndex((existing) => existing.id === template.id)

        if (index < 0) {
            store.notes_templates.push({ ...template })

            return
        }

        store.notes_templates[index] = { ...template }
    },
    notes_template_delete(call) {
        const index = store.notes_templates.findIndex((template) => template.id === call['id'])

        if (index >= 0) store.notes_templates.splice(index, 1)
    },

    people_list() {
        return store.people.map((person) => ({ ...person }))
    },
    person_save(call) {
        const person = call['person'] as Person
        const index = store.people.findIndex((existing) => existing.id === person.id)

        if (index < 0) {
            store.people.push({ ...person })

            return
        }

        store.people[index] = { ...person }
    },
    person_remove(call) {
        const index = store.people.findIndex((person) => person.id === call['id'])

        if (index >= 0) store.people.splice(index, 1)

        for (const group of store.groups) {
            group.person_ids = group.person_ids.filter((id) => id !== call['id'])
        }
    },
    groups_list() {
        return store.groups.map((group) => ({ ...group, person_ids: [...group.person_ids] }))
    },
    group_save(call) {
        const group = call['group'] as Group
        const index = store.groups.findIndex((existing) => existing.id === group.id)

        if (index < 0) {
            store.groups.push({ ...group, person_ids: [...group.person_ids] })

            return
        }

        store.groups[index] = { ...group, person_ids: [...group.person_ids] }
    },
    group_remove(call) {
        const index = store.groups.findIndex((group) => group.id === call['id'])

        if (index >= 0) store.groups.splice(index, 1)
    },

    async notes_generate_remote_streaming(call) {
        const stream_id = call['stream_id'] as string

        return await stream_text_emit(stream_id, 'notes_generate_chunk', notes_sample_markdown())
    },
    async notes_text_rewrite_streaming(call) {
        const stream_id = call['stream_id'] as string
        const text = call['text'] as string

        return await stream_text_emit(stream_id, 'notes_generate_chunk', text.toUpperCase())
    },
    async transcript_correct_streaming(call) {
        const lines = call['lines'] as string[]
        const instruction = call['instruction'] as string

        await new Promise((resolve) => setTimeout(resolve, CHUNK_DELAY_MS * 10))

        return lines.flatMap((text, index) =>
            text.includes(instruction) ? [{ index, text: text.toUpperCase() }] : [],
        )
    },
    notes_save(call) {
        store.notes_markdown.set(call['job_id'] as string, call['markdown'] as string)
    },
    notes_load(call) {
        return store.notes_markdown.get(call['job_id'] as string) ?? null
    },
    notes_export(call) {
        return call['target_path']
    },
    notes_template_generate(call) {
        return template_layout_sample(call['description'] as string)
    },
    recording_start() {
        return RECORDING_PATH_SAMPLE
    },
    recording_append() {
        return undefined
    },
    recording_finish() {
        return RECORDING_PATH_SAMPLE
    },
    recording_discard() {
        return undefined
    },
    recordings_directory_default() {
        return RECORDINGS_DIRECTORY_SAMPLE
    },
    notes_print() {
        window.print()
    },

    jobs_list() {
        return store.jobs.map((job) => {
            const imported = job.source_path === SOURCE_PATH_IMPORTED

            return {
                ...job,
                has_transcript: store.transcripts.has(job.id),
                has_notes: store.notes_markdown.has(job.id),
                duration_seconds: imported ? null : DURATION_SECONDS_SAMPLE,
            }
        })
    },
    jobs_search(call) {
        const hits = jobs_search_mock(call['query'] as string)

        assert(hits.length <= store.jobs.length, 'a job matched more than once')

        return hits
    },
    job_meta_get(call) {
        return job_find(call['job_id'] as string)
    },
    job_meta_update(call) {
        const job = job_find(call['job_id'] as string)

        if (!job) throw new Error('job not found')

        const patch = { ...(call['patch'] as JobMetaPatch) }

        if (patch.title !== undefined) patch.title = text_clean(patch.title)
        if (patch.project !== undefined) patch.project = text_clean(patch.project)

        Object.assign(job, patch)

        return { ...job }
    },
    job_delete(call) {
        const index = store.jobs.findIndex((job) => job.id === call['job_id'])

        if (index >= 0) store.jobs.splice(index, 1)
    },
    job_transcript_engines_list(call) {
        return store.transcripts.has(call['job_id'] as string) ? ['stratus.listen'] : []
    },
    job_transcript_load(call) {
        return store.transcripts.get(call['job_id'] as string) ?? null
    },
    job_transcript_save(call) {
        store.transcripts.set(call['job_id'] as string, call['transcript'] as Transcript)
    },
    job_waveform_get(call) {
        const job = store.jobs.find((candidate) => candidate.id === call['job_id'])

        if (job?.source_path === SOURCE_PATH_IMPORTED) return null

        const waveform = waveform_build(call['bar_count'] as number)

        assert(waveform.peaks.length <= WAVEFORM_BAR_COUNT_MAX, 'the waveform has too many bars')

        return waveform
    },
    job_audio_clip_get() {
        return null
    },
    job_audio_path_get() {
        return null
    },

    async transcription_remote(call) {
        const stream_id = call['stream_id'] as string | null

        const transcript = store.transcripts.get(JOB_ID_SAMPLE)
            ?? transcript_build(['No sample audio.'])

        await transcription_stream_emit(stream_id, transcript)

        const result: TranscriptionResult = { job_id: JOB_ID_SAMPLE, transcript, cached: false }

        return result
    },
    transcript_import(call) {
        const text = call['transcript_text'] as string
        const lines = text.split('\n').filter((line) => line.trim().length > 0)

        const result: TranscriptionResult = {
            job_id: JOB_ID_SAMPLE,
            transcript: transcript_build(lines),
            cached: false,
        }

        return result
    },

    task_cancel(call) {
        store.streams_cancelled.add(call['stream_id'] as string)
    },

    app_relaunch() {
        window.location.reload()
    },
    markdown_render_html(call) {
        return markdown_html_render(call['markdown'] as string)
    },

    'plugin:event|listen'(call) {
        store.listener_id_next += 1

        const listener_id = store.listener_id_next - 1

        assert(!store.listeners.has(listener_id), 'a listener id was handed out twice')

        store.listeners.set(listener_id, {
            event: call['event'] as string,
            callback_id: call['handler'] as number,
        })

        return listener_id
    },
    'plugin:event|unlisten'(call) {
        void store.listeners.delete(call['eventId'] as number)
    },
    'plugin:event|emit'(call) {
        event_emit(call['event'] as string, call['payload'])
    },
    'plugin:event|emit_to'(call) {
        event_emit(call['event'] as string, call['payload'])
    },

    'plugin:dialog|open'(call) {
        const options = call['options'] as { directory?: boolean } | undefined

        return options?.directory === true
            ? '/home/you/Music/meetings'
            : '/home/you/recordings/weekly-sync.m4a'
    },
    'plugin:dialog|save'() {
        return '/home/you/Documents/notes.docx'
    },
    'plugin:dialog|message'(call) {
        return dialog_message_show(call)
    },

    'plugin:opener|open_url'(call) {
        window.open(call['url'] as string, '_blank', 'noopener')
    },
    'plugin:opener|open_path'(call) {
        console.info('[scribe] mock openPath', call['path'])
    },
    'plugin:opener|reveal_item_in_dir'(call) {
        console.info('[scribe] mock revealItemInDir', call['path'])
    },

    'plugin:app|version'() {
        return '0.1.0-browser-mock'
    },
    'plugin:app|tauri_version'() {
        return '2.11.0'
    },
    'plugin:app|name'() {
        return 'scribe'
    },
    'plugin:app|identifier'() {
        return 'com.stratusadv.scribe'
    },

    'plugin:updater|check'() {
        return null
    },
})

function invoke_mock(command: string, call?: InvokeArgs): Promise<unknown> {
    const handler = commands[command]

    if (!handler) {
        console.warn(`[scribe] browser mock has no handler for "${command}"`)

        return Promise.resolve(null)
    }

    try {
        return Promise.resolve(handler(call ?? {}))
    } catch (error) {
        return Promise.reject(error instanceof Error ? error : new Error(String(error)))
    }
}

function callback_transform(
    callback: (payload: unknown) => void, // tigerstyle-ignore: TS034
    once: boolean,
): number {
    store.callback_id_next += 1

    const id = store.callback_id_next - 1

    assert(!store.callbacks.has(id), 'a callback id was handed out twice')

    store.callbacks.set(id, { callback, once })

    return id
}

function callback_unregister(id: number) {
    void store.callbacks.delete(id)
}

function file_source_convert(path: string): string {
    return path
}

function listener_unregister(_event: string, listener_id: number) {
    const listener = store.listeners.get(listener_id)

    if (listener) void store.callbacks.delete(listener.callback_id)

    void store.listeners.delete(listener_id)
}

function event_emit(name: string, payload: unknown) {
    for (const [listener_id, listener] of store.listeners) {
        if (listener.event !== name) continue

        const entry = store.callbacks.get(listener.callback_id)

        if (!entry) continue

        entry.callback({ event: name, id: listener_id, payload })

        if (entry.once) void store.callbacks.delete(listener.callback_id)
    }
}

async function stream_text_emit(
    stream_id: string,
    event_name: string,
    text: string,
): Promise<string> {
    const words = text.split(/(\s+)/)
    let sent = ''

    void store.streams_cancelled.delete(stream_id)

    for (const word of words) {
        if (store.streams_cancelled.has(stream_id)) break

        sent += word

        event_emit(event_name, { stream_id, text: word, done: false })
        await sleep(CHUNK_DELAY_MS)
    }

    event_emit(event_name, { stream_id, text: '', done: true })
    void store.streams_cancelled.delete(stream_id)

    return sent
}

async function transcription_stream_emit(stream_id: string | null, transcript: Transcript) {
    if (!stream_id) return

    void store.streams_cancelled.delete(stream_id)
    event_emit('transcription_stage', { stream_id, stage: 'preparing_audio' })
    await sleep(SEGMENT_DELAY_MS)
    event_emit('transcription_stage', { stream_id, stage: 'transcribing' })

    for (const segment of transcript.segments) {
        if (store.streams_cancelled.has(stream_id)) break

        event_emit('transcription_segment', {
            stream_id,
            text: segment.text,
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
        })

        await sleep(SEGMENT_DELAY_MS)
    }

    event_emit('transcription_stage', { stream_id, stage: 'done' })
    void store.streams_cancelled.delete(stream_id)
}

function dialog_message_show(call: InvokeArgs): string {
    const message = call['message'] as string
    const buttons = call['buttons'] as string | DialogButtons | undefined

    if (!buttons) {
        window.alert(message)

        return 'Ok'
    }

    const ok_label = typeof buttons === 'string' ? 'Yes' : (buttons.ok ?? 'Ok')
    const cancel_label = typeof buttons === 'string' ? 'No' : (buttons.cancel ?? 'Cancel')

    return window.confirm(message) ? ok_label : cancel_label
}

function transcript_build(lines: string[]): Transcript {
    const segments = lines.map((text, index) => ({
        text,
        start_seconds: index * SECONDS_PER_LINE,
        end_seconds: (index + 1) * SECONDS_PER_LINE,
    }))

    return { text: lines.join(' '), segments }
}

function text_clean(text: string | null): string | null {
    const trimmed = text?.trim() ?? ''

    return trimmed.length > 0 ? trimmed : null
}

function waveform_build(bar_count: number): Waveform {
    const count = Math.min(Math.max(bar_count, 1), WAVEFORM_BAR_COUNT_MAX)
    const peaks = new Array<number>(count)

    for (let index = 0; index < count; index += 1) {
        const envelope = 0.35 + 0.45 * Math.abs(Math.sin(index / 11))

        peaks[index] = envelope * (0.6 + 0.4 * Math.abs(Math.sin(index / 3.7)))
    }

    return { peaks, duration_seconds: DURATION_SECONDS_SAMPLE }
}

function job_find(job_id: string): JobMeta | null {
    return store.jobs.find((job) => job.id === job_id) ?? null
}

function jobs_search_mock(query: string): JobSearchHit[] {
    const needle = query.trim().toLowerCase()

    if (needle.length === 0) return []

    const hits: JobSearchHit[] = []

    for (const job of store.jobs) {
        const notes = store.notes_markdown.get(job.id) ?? ''
        const transcript = store.transcripts.get(job.id)?.text ?? ''
        const notes_position = notes.toLowerCase().indexOf(needle)
        const transcript_position = transcript.toLowerCase().indexOf(needle)

        if (notes_position >= 0) {
            hits.push({
                job_id: job.id,
                source: 'notes',
                snippet: snippet_around(notes, notes_position),
            })

            continue
        }

        if (transcript_position >= 0) {
            hits.push({
                job_id: job.id,
                source: 'transcript',
                snippet: snippet_around(transcript, transcript_position),
            })
        }
    }

    return hits
}

function snippet_around(text: string, position: number): string {
    const start = Math.max(0, position - SNIPPET_CHARS_BEFORE)
    const end = Math.min(text.length, position + SNIPPET_CHARS_AFTER)
    const lead = start > 0 ? '…' : ''
    const body = text.slice(start, end).replace(/\s+/g, ' ')
    const tail = end < text.length ? '…' : ''

    return `${lead}${body}${tail}`
}

function notes_sample_markdown(): string {
    return store.notes_markdown.get(JOB_ID_SAMPLE) ?? '## Notes\n\nBrowser mock output.\n'
}

function template_layout_sample(description: string): string {
    return [
        '## Summary',
        '',
        `Two or three sentences on what "${description}" covered.`,
        '',
        '## Decisions',
        '',
        '- One bullet per decision made, with who made it.',
        '',
        '## Actions',
        '',
        '| Who | What | By when |',
        '| --- | --- | --- |',
        '| The person responsible | The task in one line | The date agreed |',
        '',
    ].join('\n')
}

function sleep(milliseconds: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, milliseconds))
}

function markdown_html_render(markdown: string): string {
    const html: string[] = []
    let list_open: ListTag | null = null
    let code_open = false

    for (const line of markdown.split('\n')) {
        if (line.trimStart().startsWith('```')) {
            html.push(code_open ? '</code></pre>' : '<pre><code>')
            code_open = !code_open

            continue
        }

        if (code_open) {
            html.push(html_escape(line))

            continue
        }

        list_open = markdown_line_render(html, line, list_open)
    }

    list_close(html, list_open)

    if (code_open) html.push('</code></pre>')

    return html.join('\n')
}

function markdown_line_render(
    html: string[],
    line: string,
    list_open: ListTag | null,
): ListTag | null {
    const heading = /^(#{1,6})\s+(.*)$/.exec(line)
    const bullet = /^\s*[-*]\s+(.*)$/.exec(line)
    const numbered = /^\s*\d+\.\s+(.*)$/.exec(line)
    const quote = /^>\s?(.*)$/.exec(line)

    if (heading) {
        const level = (heading[1] ?? '#').length

        list_close(html, list_open)
        html.push(`<h${level}>${inline_html_render(heading[2] ?? '')}</h${level}>`)

        return null
    }

    if (bullet) {
        const list = list_open_ensure(html, list_open, 'ul')

        html.push(`<li>${inline_html_render(bullet[1] ?? '')}</li>`)

        return list
    }

    if (numbered) {
        const list = list_open_ensure(html, list_open, 'ol')

        html.push(`<li>${inline_html_render(numbered[1] ?? '')}</li>`)

        return list
    }

    list_close(html, list_open)

    if (quote) {
        html.push(`<blockquote>${inline_html_render(quote[1] ?? '')}</blockquote>`)

        return null
    }

    if (line.trim().length === 0) return null

    if (/^\s*(-{3,}|\*{3,})\s*$/.test(line)) {
        html.push('<hr/>')

        return null
    }

    html.push(`<p>${inline_html_render(line)}</p>`)

    return null
}

function list_open_ensure(html: string[], current: ListTag | null, wanted: ListTag): ListTag {
    if (current === wanted) return current
    if (current) html.push(`</${current}>`)

    html.push(`<${wanted}>`)

    return wanted
}

function list_close(html: string[], current: ListTag | null) {
    if (current) html.push(`</${current}>`)
}

function inline_html_render(text: string): string {
    return html_escape(text)
        .replace(/`([^`]+)`/g, '<code>$1</code>')
        .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
        .replace(/(^|[^*])\*([^*]+)\*/g, '$1<em>$2</em>')
        .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>')
}

function html_escape(text: string): string {
    return text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
}
