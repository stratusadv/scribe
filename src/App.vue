<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import Sidebar from './components/Sidebar.vue'
import JobsManager from './components/JobsManager.vue'
import StepTranscribe from './components/StepTranscribe.vue'
import StepImport from './components/StepImport.vue'
import StepMeeting from './components/StepMeeting.vue'
import StepNotes from './components/StepNotes.vue'
import PeopleManager from './components/PeopleManager.vue'
import TemplatesManager from './components/TemplatesManager.vue'
import SettingsManager from './components/SettingsManager.vue'
import AppDialogHost from './components/AppDialogHost.vue'
import NoticeBanner from './components/NoticeBanner.vue'
import { use_pipeline } from './composables/use_pipeline'
import { use_endpoints } from './composables/use_endpoints'
import { use_notes_templates } from './composables/use_notes_templates'
import { use_people } from './composables/use_people'
import { use_jobs } from './composables/use_jobs'
import { use_service } from './composables/use_service'
import { use_settings } from './composables/use_settings'
import { use_network } from './composables/use_network'
import { use_drag_drop } from './composables/use_drag_drop'
import { use_theme } from './composables/use_theme'
import { use_update } from './composables/use_update'


const { view_current, view_reopen_count } = use_pipeline()
const { refresh: endpoints_refresh } = use_endpoints()
const { refresh: templates_refresh } = use_notes_templates()
const { refresh: people_refresh } = use_people()
const { refresh: jobs_refresh } = use_jobs()
const { refresh: settings_refresh } = use_settings()
const { refresh: service_refresh } = use_service()
const { online } = use_network()
const { dragging_over } = use_drag_drop()
const { phase: update_phase, status_text: update_status_text, update_check_startup } = use_update()


use_theme()

const offline_banner_dismissed = ref(false)
const update_banner_dismissed = ref(false)
const update_banner_visible = computed(
    () => update_phase.value === 'downloading' || update_phase.value === 'installing',
)

const ready = ref<boolean>(false)

const view_components = {
    jobs: JobsManager,
    transcribe: StepTranscribe,
    import: StepImport,
    details: StepMeeting,
    notes: StepNotes,
    templates: TemplatesManager,
    people: PeopleManager,
    settings: SettingsManager,
}

const current_component = computed(() => view_components[view_current.value])
const view_key = computed(() => `${view_current.value}-${view_reopen_count.value}`)

watch(online, () => {
    offline_banner_dismissed.value = false
})

watch(update_banner_visible, () => {
    update_banner_dismissed.value = false
})

function editor_load() {
    void import('./components/RichEditor.vue')
}

function editor_prefetch() {
    if (typeof window.requestIdleCallback === 'function') {
        window.requestIdleCallback(editor_load)

        return
    }

    setTimeout(editor_load, 0)
}

onMounted(async () => {
    await Promise.all([
        settings_refresh(),
        service_refresh(),
        endpoints_refresh(),
        templates_refresh(),
        people_refresh(),
        jobs_refresh(),
        document.fonts.load('1em "Geist"'),
        document.fonts.load('italic 1em "Geist"'),
        document.fonts.load('1em "Geist Mono"'),
    ])

    ready.value = true

    editor_prefetch()

    void update_check_startup()
})
</script>

<template>
    <div
        class="flex flex-col h-screen overflow-hidden app-shell"
        :data-ready="ready ? 'true' : 'false'"
    >
        <NoticeBanner
            v-if="!online && !offline_banner_dismissed"
            kind="warning"
            class="banner-shell"
            @dismiss="offline_banner_dismissed = true"
        >
            You appear to be offline. An internet connection is needed to transcribe and to write notes.
        </NoticeBanner>
        <NoticeBanner
            v-if="update_banner_visible && !update_banner_dismissed"
            kind="info"
            class="banner-shell"
            @dismiss="update_banner_dismissed = true"
        >
            {{ update_status_text }}
        </NoticeBanner>
        <div class="flex flex-1 overflow-hidden">
            <Sidebar />
            <main class="flex-1 overflow-y-auto">
                <div class="app-content px-5 py-6 sm:px-8 sm:py-8 lg:px-12 lg:py-10">
                    <KeepAlive include="StepNotes">
                        <component :is="current_component" :key="view_key" />
                    </KeepAlive>
                </div>
            </main>
        </div>
        <div v-if="dragging_over" class="drop-overlay">
            Drop audio or video files. A single file opens for review, and several files are transcribed in a batch.
        </div>
        <AppDialogHost />
    </div>
</template>
