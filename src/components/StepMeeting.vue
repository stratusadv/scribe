<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { ipc } from '../lib/ipc'
import { use_jobs } from '../composables/use_jobs'
import { use_notes_templates } from '../composables/use_notes_templates'
import { person_name_full, use_people } from '../composables/use_people'
import { use_pipeline } from '../composables/use_pipeline'
import { seconds_to_clock } from '../lib/duration'
import { speaker_color, speaker_label, speakers_summarize } from '../lib/speakers'
import NoticeBanner from './NoticeBanner.vue'
import PeoplePicker from './PeoplePicker.vue'
import SearchSelect from './SearchSelect.vue'
import type { SearchSelectOption } from './SearchSelect.vue'
import StepBar from './StepBar.vue'
import TranscriptPane from './TranscriptPane.vue'
import type { JobMetaPatch, Person, SpeakerLink } from '../types'


type PickerTarget = 'present' | 'mentioned'
type SaveStatus = 'idle' | 'pending' | 'saving' | 'saved'
const AUTOSAVE_DEBOUNCE_MS = 800

const PICKER_HINT_MENTIONED = 'Tick people who came up in conversation but were not there. '
    + 'The AI will recognise them by name and role.'

const PICKER_HINT_PRESENT = 'Tick each person who was there. The AI uses their role and '
    + 'description to tell who is speaking.'

const {
    job_id_current,
    meta_current,
    template_id_selected,
    transcript,
    meta_refresh,
    template_id_ensure,
    view_set,
} = use_pipeline()

const { refresh: jobs_refresh } = use_jobs()
const { people_sorted, person_by_id } = use_people()
const { templates } = use_notes_templates()
const title_input = ref('')
const person_ids_current = ref<string[]>([])
const person_ids_mentioned = ref<string[]>([])
const attendees_legacy = ref<string[]>([])
const picker_target = ref<PickerTarget | null>(null)
const project_input = ref('')
const tags_current = ref<string[]>([])
const speaker_links_current = ref<SpeakerLink[]>([])
const speaker_open = ref<number | null>(null)
const status = ref<SaveStatus>('idle')
const error_message = ref<string | null>(null)
let timer_id: ReturnType<typeof setTimeout> | null = null
let job_id_pending: string | null = null
let patch_pending: JobMetaPatch | null = null
let patch_synced = ''


function people_resolve(ids: string[]): Person[] {
    return ids.map(person_by_id).filter((person): person is Person => person !== null)
}

const people_present = computed(() => people_resolve(person_ids_current.value))
const people_mentioned = computed(() => people_resolve(person_ids_mentioned.value))
const speakers = computed(() => (transcript.value ? speakers_summarize(transcript.value) : []))

const person_options = computed<SearchSelectOption[]>(() =>
    people_sorted.value.map((person) => ({
        value: person.id,
        text: person.role ? `${person_name_full(person)} (${person.role})` : person_name_full(person),
    })),
)

const template_options = computed<SearchSelectOption[]>(() =>
    templates.value.map((template) => ({ value: template.id, text: template.name })),
)

const picker_ids = computed({
    get: () => {
        if (picker_target.value === 'mentioned') return person_ids_mentioned.value

        return person_ids_current.value
    },
    set: (ids: string[]) => {
        if (picker_target.value === 'mentioned') {
            person_ids_mentioned.value = ids

            return
        }

        person_ids_current.value = ids
    },
})

const picker_title = computed(() =>
    picker_target.value === 'mentioned' ? 'Who was talked about?' : 'Who was in the meeting?',
)

const picker_hint = computed(() =>
    picker_target.value === 'mentioned' ? PICKER_HINT_MENTIONED : PICKER_HINT_PRESENT,
)

const can_continue = computed(
    () => status.value !== 'saving' && template_id_selected.value !== null,
)

onMounted(() => {
    template_id_ensure()
})

onUnmounted(() => {
    if (timer_id === null) return

    clearTimeout(timer_id)

    void autosave_flush().then(jobs_refresh)
})

watch(
    () => job_id_current.value,
    () => {
        speaker_open.value = null
    },
)

watch(
    () => meta_current.value,
    (meta) => {
        if (!meta) {
            title_input.value = ''
            person_ids_current.value = []
            person_ids_mentioned.value = []
            attendees_legacy.value = []
            project_input.value = ''
            tags_current.value = []
            speaker_links_current.value = []
            patch_synced = ''

            return
        }

        title_input.value = meta.title ?? ''
        person_ids_current.value = [...meta.person_ids]
        person_ids_mentioned.value = [...meta.person_ids_mentioned]
        attendees_legacy.value = meta.person_ids.length === 0 ? [...meta.attendees] : []
        project_input.value = meta.project ?? ''
        tags_current.value = [...meta.tags]
        speaker_links_current.value = meta.speaker_links.map((link) => ({ ...link }))
        patch_synced = JSON.stringify(patch_build())
    },
    { immediate: true },
)

watch(
    [
        title_input,
        person_ids_current,
        person_ids_mentioned,
        project_input,
        tags_current,
        speaker_links_current,
    ],
    () => {
        if (!job_id_current.value) return
        if (!autosave_pending() && JSON.stringify(patch_build()) === patch_synced) return

        autosave_schedule()
    },
    { deep: true },
)

function patch_build(): JobMetaPatch {
    const people = people_present.value

    return {
        title: title_input.value.trim(),
        attendees: people.length > 0 ? people.map(person_name_full) : attendees_legacy.value,
        person_ids: people.map((person) => person.id),
        person_ids_mentioned: people_mentioned.value.map((person) => person.id),
        project: project_input.value.trim(),
        tags: [...tags_current.value],
        speaker_links: speaker_links_current.value.map((link) => ({ ...link })),
    }
}

function speaker_person_id(speaker: number): string {
    return speaker_links_current.value.find((link) => link.speaker === speaker)?.person_id ?? ''
}

function speaker_assign(speaker: number, person_id: string) {
    const others = speaker_links_current.value.filter((link) => link.speaker !== speaker)

    speaker_links_current.value = person_id.length === 0
        ? others
        : [...others, { speaker, person_id }]

    if (person_id.length > 0 && !person_ids_current.value.includes(person_id)) {
        person_ids_current.value = [...person_ids_current.value, person_id]
    }
}

function person_remove(id: string) {
    person_ids_current.value = person_ids_current.value.filter((selected) => selected !== id)
}

function person_mentioned_remove(id: string) {
    person_ids_mentioned.value = person_ids_mentioned.value.filter((selected) => selected !== id)
}

function autosave_pending(): boolean {
    return timer_id !== null
}

function autosave_schedule() {
    status.value = 'pending'
    job_id_pending = job_id_current.value
    patch_pending = patch_build()

    if (timer_id !== null) clearTimeout(timer_id)

    timer_id = setTimeout(() => {
        void autosave_flush()
    }, AUTOSAVE_DEBOUNCE_MS)
}

async function autosave_flush() {
    const job_id = job_id_pending
    const patch = patch_pending

    timer_id = null
    job_id_pending = null
    patch_pending = null

    if (!job_id || !patch) return

    status.value = 'saving'
    error_message.value = null

    try {
        const updated = await ipc.job_meta_update(job_id, patch)

        if (job_id_current.value !== job_id || autosave_pending()) return

        meta_current.value = updated
        status.value = 'saved'
    } catch (error) {
        status.value = 'idle'
        error_message.value = String(error)

        await meta_refresh()
    }
}

function back() {
    view_set('transcribe')
}

async function next() {
    if (timer_id !== null) {
        clearTimeout(timer_id)

        await autosave_flush()
    }

    view_set('notes')
}
</script>

<template>
    <section class="view-fill step-view">
        <StepBar />

        <NoticeBanner v-if="error_message" @dismiss="error_message = null">
            {{ error_message }}
        </NoticeBanner>

        <div class="step-form flex-1">
            <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
                <div class="step-field">
                    <label class="label" for="details-title">Title</label>
                    <input
                        id="details-title"
                        v-model="title_input"
                        type="search"
                        class="input"
                    />
                </div>

                <div class="step-field">
                    <label class="label" for="details-template">Template</label>
                    <SearchSelect
                        id="details-template"
                        label="Template"
                        placeholder="Choose a template"
                        :options="template_options"
                        :value="template_id_selected"
                        @update:value="template_id_selected = $event"
                    />
                    <p v-if="templates.length === 0" class="meta">
                        There are no templates yet.
                        <button type="button" class="underline" @click="view_set('templates')">
                            Add one
                        </button>
                    </p>
                </div>
            </div>

            <div class="step-field">
                <span class="label">In the meeting</span>
                <div class="meeting-details-people">
                    <button
                        type="button"
                        class="meeting-details-add"
                        @click="picker_target = 'present'"
                    >
                        + Add
                    </button>
                    <span v-for="person in people_present" :key="person.id" class="pill tag-chip tag-chip-person">
                        {{ person_name_full(person) }}<template v-if="person.role"> ({{ person.role }})</template>
                        <button
                            type="button"
                            class="tag-chip-remove"
                            :aria-label="`Remove ${person_name_full(person)}`"
                            @click="person_remove(person.id)"
                        >
                            <svg
                                viewBox="0 0 24 24"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2.5"
                                stroke-linecap="round"
                                aria-hidden="true"
                            >
                                <path d="M6 6l12 12" />
                                <path d="M18 6L6 18" />
                            </svg>
                        </button>
                    </span>
                </div>
                <p v-if="attendees_legacy.length > 0" class="meta">
                    These names are from before People existed: {{ attendees_legacy.join(', ') }}
                </p>
            </div>

            <div v-if="speakers.length > 0" class="step-field">
                <span class="label">Speakers</span>
                <div class="meeting-speakers">
                    <div v-for="summary in speakers" :key="summary.speaker" class="meeting-speaker">
                        <div class="meeting-speaker-head">
                            <span
                                class="meeting-speaker-label"
                                :style="{ color: speaker_color(summary.speaker) }"
                            >
                                {{ speaker_label(summary.speaker) }}
                            </span>
                            <span class="meta">{{ seconds_to_clock(summary.seconds) }} of talking</span>
                        </div>
                        <div class="meeting-speaker-actions">
                            <SearchSelect
                                empty_text="Not assigned"
                                :color="speaker_color(summary.speaker)"
                                :label="`Who is ${speaker_label(summary.speaker)}`"
                                :options="person_options"
                                :value="speaker_person_id(summary.speaker)"
                                @update:value="speaker_assign(summary.speaker, $event)"
                            />
                            <button
                                type="button"
                                class="btn-default"
                                :aria-label="`Transcript of ${speaker_label(summary.speaker)}`"
                                @click="speaker_open = summary.speaker"
                            >
                                Transcript
                            </button>
                        </div>
                    </div>
                </div>
            </div>

            <div class="step-field">
                <span class="label">Mentioned</span>
                <div class="meeting-details-people">
                    <button
                        type="button"
                        class="meeting-details-add"
                        @click="picker_target = 'mentioned'"
                    >
                        + Add
                    </button>
                    <span v-for="person in people_mentioned" :key="person.id" class="pill tag-chip tag-chip-person">
                        {{ person_name_full(person) }}<template v-if="person.role"> ({{ person.role }})</template>
                        <button
                            type="button"
                            class="tag-chip-remove"
                            :aria-label="`Remove ${person_name_full(person)}`"
                            @click="person_mentioned_remove(person.id)"
                        >
                            <svg
                                viewBox="0 0 24 24"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2.5"
                                stroke-linecap="round"
                                aria-hidden="true"
                            >
                                <path d="M6 6l12 12" />
                                <path d="M18 6L6 18" />
                            </svg>
                        </button>
                    </span>
                </div>
            </div>
        </div>

        <footer class="step-footer">
            <div class="step-footer-start">
                <button class="btn-default" @click="back">Back</button>
            </div>
            <div class="actions-row">
                <button class="btn-primary" :disabled="!can_continue" @click="next">Next</button>
            </div>
        </footer>

        <PeoplePicker
            v-if="picker_target !== null"
            v-model:ids_selected="picker_ids"
            :title="picker_title"
            :hint="picker_hint"
            @close="picker_target = null"
        />

        <Teleport v-if="speaker_open !== null" to="body">
            <div
                class="app-dialog-backdrop"
                role="dialog"
                aria-modal="true"
                @click.self="speaker_open = null"
                @keydown.esc="speaker_open = null"
            >
                <div class="app-dialog speaker-lines">
                    <button
                        type="button"
                        class="app-dialog-close"
                        aria-label="Close"
                        title="Close"
                        @click="speaker_open = null"
                    >
                        <svg
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="1.75"
                            stroke-linecap="round"
                            aria-hidden="true"
                        >
                            <path d="M6 6l12 12" />
                            <path d="M18 6L6 18" />
                        </svg>
                    </button>
                    <h3 class="app-dialog-title" :style="{ color: speaker_color(speaker_open) }">
                        {{ speaker_label(speaker_open) }}
                    </h3>
                    <p class="app-dialog-message">
                        Play a few lines to hear who this is, then pick them below.
                    </p>
                    <SearchSelect
                        empty_text="Not assigned"
                        :color="speaker_color(speaker_open)"
                        :label="`Who is ${speaker_label(speaker_open)}`"
                        :options="person_options"
                        :value="speaker_person_id(speaker_open)"
                        @update:value="speaker_assign(speaker_open, $event)"
                    />
                    <div class="speaker-lines-transcript">
                        <TranscriptPane :transcript="transcript" :speaker="speaker_open" />
                    </div>
                    <div class="app-dialog-actions">
                        <button type="button" class="btn-primary" @click="speaker_open = null">
                            Done
                        </button>
                    </div>
                </div>
            </div>
        </Teleport>
    </section>
</template>
