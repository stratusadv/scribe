<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { use_jobs } from '../composables/use_jobs'
import { job_duration_text, job_status } from '../lib/job_status'
import RowMenu from './RowMenu.vue'
import StarButton from './StarButton.vue'
import type { RowMenuItem } from './RowMenu.vue'
import type { JobListing, Waveform } from '../types'


const WAVEFORM_BAR_HEIGHT_MIN = 4
const WAVEFORM_BAR_PITCH = 3
const WAVEFORM_BAR_WIDTH = 2
const WAVEFORM_HEIGHT = 100

const props = defineProps<{
    job: JobListing
    title: string
    when: string
    people: string | null
    snippet: string | null
    menu_items: RowMenuItem[]
}>()

const emit = defineEmits<{
    open: []
    favourite: []
    menu: [id: string]
}>()

const { waveform_load } = use_jobs()
const waveform = ref<Waveform | null>(null)

const wave_bars = computed(() =>
    (waveform.value?.peaks ?? []).map((peak, index) => ({
        x: index * WAVEFORM_BAR_PITCH,
        height: Math.max(WAVEFORM_BAR_HEIGHT_MIN, peak * WAVEFORM_HEIGHT),
    })),
)

const wave_view_box = computed(
    () => `0 0 ${Math.max(1, wave_bars.value.length) * WAVEFORM_BAR_PITCH} ${WAVEFORM_HEIGHT}`,
)

const duration_text = computed(() => job_duration_text(props.job))
const status = computed(() => job_status(props.job))


onMounted(async () => {
    try {
        waveform.value = await waveform_load(props.job.id)
    } catch (error) {
        console.warn('[recordings] waveform unavailable', props.job.id, error)
    }
})
</script>

<template>
    <article
        class="recording-tile card"
        tabindex="0"
        @click="emit('open')"
        @keydown.enter="emit('open')"
    >
        <div class="recording-tile-band">
            <svg
                class="recording-tile-wave"
                :viewBox="wave_view_box"
                preserveAspectRatio="none"
                aria-hidden="true"
            >
                <rect
                    v-for="bar in wave_bars"
                    :key="bar.x"
                    :x="bar.x"
                    :y="(WAVEFORM_HEIGHT - bar.height) / 2"
                    :width="WAVEFORM_BAR_WIDTH"
                    :height="bar.height"
                    rx="1"
                />
            </svg>
            <div class="recording-tile-tools" @click.stop @keydown.enter.stop>
                <StarButton
                    :favourite="props.job.favourite"
                    :title="props.title"
                    @toggle="emit('favourite')"
                />
                <span class="recording-tile-menu">
                    <RowMenu
                        :label="props.title"
                        :items="props.menu_items"
                        @select="emit('menu', $event)"
                    />
                </span>
            </div>
            <span v-if="duration_text" class="recording-tile-duration">{{ duration_text }}</span>
        </div>

        <div class="recording-tile-body">
            <h3 class="recording-tile-title">{{ props.title }}</h3>
            <div class="recording-tile-people meta">
                {{ props.snippet ?? props.people ?? 'No participants' }}
            </div>
            <div class="recording-tile-foot meta">
                <span>{{ props.when }}</span>
                <span class="pill recording-tile-status" :data-state="status.state">
                    {{ status.text }}
                </span>
            </div>
        </div>
    </article>
</template>
