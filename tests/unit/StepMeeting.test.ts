import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import StepMeeting from '../../src/components/StepMeeting.vue'
import { use_notes_templates } from '../../src/composables/use_notes_templates'
import { use_people } from '../../src/composables/use_people'
import { use_pipeline } from '../../src/composables/use_pipeline'
import { ipc } from '../../src/lib/ipc'
import { job_meta_build, person_build, transcript_build } from './fixtures'
import type { VueWrapper } from '@vue/test-utils'


vi.mock('../../src/lib/ipc')
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => `asset://${path}` }))

const JOHN = person_build({ id: 'john', name_first: 'John', name_last: 'Doe', role: 'Developer' })
const JANE = person_build({ id: 'jane', name_first: 'Jane', name_last: 'Doe', role: 'Owner' })
const mounted: VueWrapper[] = []
const pipeline = use_pipeline()
const { templates } = use_notes_templates()

function transcript_with_speakers() {
    const transcript = transcript_build(['short', 'other voice', 'a much longer line here'])
    const speakers = [1, 0, 1]

    transcript.segments = transcript.segments.map((segment, index) => ({
        ...segment,
        speaker: speakers[index] ?? null,
        end_seconds: index === 2 ? 45 : segment.end_seconds,
    }))

    return transcript
}

function mount_step(): VueWrapper {
    const wrapper = mount(StepMeeting, { global: { stubs: { StepBar: true } } })

    mounted.push(wrapper)

    return wrapper
}

function cards(wrapper: VueWrapper) {
    return wrapper.findAll('.meeting-speaker')
}

beforeEach(async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    vi.mocked(ipc.people_list).mockResolvedValue([JOHN, JANE])
    vi.mocked(ipc.groups_list).mockResolvedValue([])
    vi.mocked(ipc.jobs_list).mockResolvedValue([])
    vi.mocked(ipc.job_audio_path_get).mockResolvedValue(null)
    vi.mocked(ipc.job_meta_update).mockImplementation((_id, patch) =>
        Promise.resolve({ ...job_meta_build({ id: 'job-1' }), ...patch } as never),
    )

    await use_people().refresh()

    pipeline.pipeline_reset()
    pipeline.job_id_current.value = 'job-1'
    pipeline.transcript.value = transcript_with_speakers()
    pipeline.meta_current.value = job_meta_build({ id: 'job-1', person_ids: ['jane'] })
    pipeline.template_id_selected.value = 'meeting'

    templates.value = [
        { id: 'meeting', name: 'Meeting', description: '', instructions: '', edited: false },
    ]
})

afterEach(() => {
    for (const wrapper of mounted) wrapper.unmount()

    mounted.length = 0
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
    vi.useRealTimers()
})

describe('StepMeeting speakers', () => {
    it('lists each detected speaker with its talk time', async () => {
        const wrapper = mount_step()

        await flushPromises()

        expect(cards(wrapper).map((card) => card.get('.meeting-speaker-label').text())).toEqual([
            'Speaker A',
            'Speaker B',
        ])
        expect(cards(wrapper)[1]?.text()).toContain('0:35 of talking')
    })

    it('opens every line of one speaker with the assignment picker beside them', async () => {
        const wrapper = mount_step()

        await flushPromises()
        await wrapper.get('button[aria-label="Transcript of Speaker B"]').trigger('click')
        await flushPromises()

        const lines = [...document.body.querySelectorAll('.speaker-lines .transcript-pane-row-text')]

        expect(lines.map((line) => line.textContent)).toEqual(['short', 'a much longer line here'])
        expect(document.body.querySelectorAll('.speaker-lines input[aria-label="Who is Speaker B"]'))
            .toHaveLength(1)
    })

    it('assigning a speaker links the person, adds them to the meeting, and autosaves', async () => {
        vi.useFakeTimers()

        const wrapper = mount_step()

        await flushPromises()

        const field = wrapper.get('input[aria-label="Who is Speaker B"]')

        await field.trigger('focus')
        await field.setValue('john')

        expect(wrapper.findAll('[role="option"]').map((option) => option.text()))
            .toEqual(['John Doe (Developer)'])

        await field.trigger('keydown', { key: 'Enter' })
        await vi.advanceTimersByTimeAsync(1000)

        expect(ipc.job_meta_update).toHaveBeenCalledWith('job-1', expect.objectContaining({
            person_ids: ['jane', 'john'],
            speaker_links: [{ speaker: 1, person_id: 'john' }],
        }))

        expect((field.element as HTMLInputElement).value).toBe('John Doe (Developer)')

        await field.trigger('focus')
        await wrapper.get('[role="option"]').trigger('mousedown')
        await vi.advanceTimersByTimeAsync(1000)

        expect(ipc.job_meta_update).toHaveBeenLastCalledWith('job-1', expect.objectContaining({
            person_ids: ['jane', 'john'],
            speaker_links: [],
        }))
    })

    it('plays one line of a speaker from the listen dialog', async () => {
        const fetch_mock = vi.fn().mockResolvedValue({ ok: true, blob: () => Promise.resolve(new Blob(['wav'])) })
        const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()

        vi.mocked(ipc.job_audio_path_get).mockResolvedValue('/tmp/take.wav')
        vi.mocked(ipc.job_audio_clip_get).mockResolvedValue('/tmp/clip.wav')
        vi.stubGlobal('fetch', fetch_mock)
        vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:clip')
        vi.spyOn(URL, 'revokeObjectURL').mockReturnValue(undefined)

        const wrapper = mount_step()

        await flushPromises()
        await wrapper.get('button[aria-label="Transcript of Speaker B"]').trigger('click')
        await flushPromises()

        const buttons = document.body.querySelectorAll<HTMLButtonElement>('.speaker-lines .transcript-pane-play')

        expect(buttons).toHaveLength(2)

        buttons[1]?.click()
        await flushPromises()

        expect(ipc.job_audio_clip_get).toHaveBeenCalledWith('job-1', 20, 45)
        expect(fetch_mock).toHaveBeenCalledWith('asset:///tmp/clip.wav')
        expect(play).toHaveBeenCalledOnce()
    })
})
