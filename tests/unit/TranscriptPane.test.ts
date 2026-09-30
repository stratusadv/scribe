import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import TranscriptPane from '../../src/components/TranscriptPane.vue'
import { use_pipeline } from '../../src/composables/use_pipeline'
import { ipc } from '../../src/lib/ipc'
import { transcript_build } from './fixtures'
import type { Transcript } from '../../src/types'
import type { VueWrapper } from '@vue/test-utils'


vi.mock('../../src/lib/ipc')
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => `asset://${path}` }))

const LINES = [
    'Alright, everyone is here.',
    'The retry work (v2) is close.',
    '   ',
    'Sam is drafting the banner copy.',
]

const mounted: VueWrapper[] = []
const pipeline = use_pipeline()

function mount_pane(
    transcript: Transcript | null = transcript_build(LINES),
    segment_indexes_highlighted?: number[],
) {
    const wrapper = mount(TranscriptPane, {
        props: segment_indexes_highlighted
            ? { transcript, segment_indexes_highlighted }
            : { transcript },
        attachTo: document.body,
    })

    mounted.push(wrapper)

    return wrapper
}

function row_texts(wrapper: ReturnType<typeof mount_pane>) {
    return wrapper.findAll('.transcript-pane-row-text').map((row) => row.text())
}

beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(ipc.job_audio_path_get).mockResolvedValue(null)
    pipeline.pipeline_reset()
    pipeline.job_id_current.value = 'job-1'
    pipeline.transcript.value = transcript_build(LINES)
})

afterEach(() => {
    for (const wrapper of mounted) wrapper.unmount()

    mounted.length = 0
    vi.restoreAllMocks()
    document.body.innerHTML = ''
})

describe('TranscriptPane', () => {
    it('labels rows by line number when no segment carries timing', async () => {
        const untimed = transcript_build(LINES)

        for (const segment of untimed.segments) {
            segment.start_seconds = 0
            segment.end_seconds = 0
        }

        const wrapper = mount_pane(untimed)

        await flushPromises()

        expect(row_texts(wrapper)).toHaveLength(3)

        expect(wrapper.findAll('.transcript-pane-row-time').map((time) => time.text())).toEqual([
            'Line 1',
            'Line 2',
            'Line 4',
        ])
    })

    it('renders one row per non-blank segment with its start time', async () => {
        const wrapper = mount_pane()

        await flushPromises()

        expect(ipc.job_audio_path_get).toHaveBeenCalledWith('job-1')

        expect(row_texts(wrapper)).toEqual([
            'Alright, everyone is here.',
            'The retry work (v2) is close.',
            'Sam is drafting the banner copy.',
        ])

        expect(wrapper.findAll('.transcript-pane-row-time').map((time) => time.text())).toEqual([
            '0:00',
            '0:10',
            '0:30',
        ])
    })

    it('explains when there is no transcript', async () => {
        const wrapper = mount_pane(null)

        await flushPromises()

        expect(wrapper.text()).toContain('There is no transcript yet.')
        expect(wrapper.find('.transcript-pane-list').exists()).toBe(false)
    })

    it('narrows the rows to those matching the filter, ignoring case', async () => {
        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.get('.transcript-pane-filter').setValue('RETRY')

        expect(row_texts(wrapper)).toEqual(['The retry work (v2) is close.'])
    })

    it('treats regex characters in the filter literally', async () => {
        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.get('.transcript-pane-filter').setValue('(v2)')

        expect(row_texts(wrapper)).toEqual(['The retry work (v2) is close.'])

        await wrapper.get('.transcript-pane-filter').setValue('.*')

        expect(row_texts(wrapper)).toEqual([])
        expect(wrapper.text()).toContain('No segments match.')
    })

    it('shows every row again when the filter is cleared or blank', async () => {
        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.get('.transcript-pane-filter').setValue('banner')

        expect(row_texts(wrapper)).toHaveLength(1)

        await wrapper.get('.transcript-pane-filter').setValue('   ')

        expect(row_texts(wrapper)).toHaveLength(3)

        await wrapper.get('.transcript-pane-filter').setValue('')

        expect(row_texts(wrapper)).toHaveLength(3)
    })

    it('marks highlighted segments by their original index', async () => {
        const wrapper = mount_pane(transcript_build(LINES), [3])

        await flushPromises()

        const rows = wrapper.findAll('.transcript-pane-row')

        expect(rows.map((row) => row.attributes('data-highlighted'))).toEqual([
            'false',
            'false',
            'true',
        ])
    })

    it('copies a row to the clipboard and flashes Copied', async () => {
        vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })

        const write_text = vi.fn().mockResolvedValue(undefined)

        vi.stubGlobal('navigator', { clipboard: { writeText: write_text } })

        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-row')[1]?.trigger('click')
        await flushPromises()

        expect(write_text).toHaveBeenCalledWith('The retry work (v2) is close.')
        expect(wrapper.findAll('.transcript-pane-row')[1]?.attributes('data-copied')).toBe('true')
        expect(wrapper.findAll('.pill-active')[0]?.text()).toBe('Copied')

        vi.advanceTimersByTime(1500)
        await wrapper.vm.$nextTick()

        expect(wrapper.findAll('.transcript-pane-row')[1]?.attributes('data-copied')).toBe('false')
        expect(wrapper.find('.pill-active').exists()).toBe(false)

        vi.useRealTimers()
        vi.unstubAllGlobals()
    })

    it('warns instead of flashing when the clipboard write fails', async () => {
        const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)

        vi.stubGlobal('navigator', {
            clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
        })

        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-row')[0]?.trigger('click')
        await flushPromises()

        expect(warn).toHaveBeenCalledWith('[transcript] clipboard write failed', expect.any(Error))
        expect(wrapper.find('.pill-active').exists()).toBe(false)

        vi.unstubAllGlobals()
    })

    it('opens a textarea with the row text when the pencil is clicked', async () => {
        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-edit')[1]?.trigger('click')

        const textarea = wrapper.get('textarea')

        expect(textarea.element.value).toBe('The retry work (v2) is close.')
        expect(wrapper.findAll('.transcript-pane-row')[1]?.attributes('data-editing')).toBe('true')
        expect(document.activeElement).toBe(textarea.element)
    })

    it('commits an edit through the pipeline with the original segment index', async () => {
        vi.mocked(ipc.job_transcript_save).mockResolvedValue(undefined)

        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-edit')[2]?.trigger('click')
        await wrapper.get('textarea').setValue('  Sam drafts the copy.  ')
        await wrapper.get('textarea').trigger('keydown', { key: 'Enter' })
        await flushPromises()

        expect(pipeline.transcript.value?.segments[3]?.text).toBe('Sam drafts the copy.')

        expect(pipeline.transcript.value?.text).toBe(
            'Alright, everyone is here. The retry work (v2) is close. Sam drafts the copy.',
        )

        expect(ipc.job_transcript_save).toHaveBeenCalledWith('job-1', pipeline.transcript.value)
        expect(wrapper.find('textarea').exists()).toBe(false)
    })

    it('commits on blur and through the pencil a second time', async () => {
        vi.mocked(ipc.job_transcript_save).mockResolvedValue(undefined)

        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-edit')[0]?.trigger('click')
        await wrapper.get('textarea').setValue('Blurred')
        await wrapper.get('textarea').trigger('blur')
        await flushPromises()

        expect(ipc.job_transcript_save).toHaveBeenCalledTimes(1)
        expect(pipeline.transcript.value?.segments[0]?.text).toBe('Blurred')

        await wrapper.findAll('.transcript-pane-edit')[1]?.trigger('click')
        await wrapper.get('textarea').setValue('Penciled')
        await wrapper.findAll('.transcript-pane-edit')[1]?.trigger('click')
        await flushPromises()

        expect(ipc.job_transcript_save).toHaveBeenCalledTimes(2)
        expect(pipeline.transcript.value?.segments[1]?.text).toBe('Penciled')
    })

    it('does not save an unchanged or blank edit', async () => {
        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-edit')[0]?.trigger('click')
        await wrapper.get('textarea').trigger('keydown', { key: 'Enter' })
        await flushPromises()

        expect(ipc.job_transcript_save).not.toHaveBeenCalled()

        await wrapper.findAll('.transcript-pane-edit')[0]?.trigger('click')
        await wrapper.get('textarea').setValue('   ')
        await wrapper.get('textarea').trigger('keydown', { key: 'Enter' })
        await flushPromises()

        expect(ipc.job_transcript_save).not.toHaveBeenCalled()
        expect(pipeline.transcript.value?.segments[0]?.text).toBe('Alright, everyone is here.')
    })

    it('discards the edit on escape', async () => {
        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-edit')[0]?.trigger('click')
        await wrapper.get('textarea').setValue('Changed')
        await wrapper.get('textarea').trigger('keydown', { key: 'Escape' })

        expect(wrapper.find('textarea').exists()).toBe(false)
        expect(row_texts(wrapper)[0]).toBe('Alright, everyone is here.')
        expect(ipc.job_transcript_save).not.toHaveBeenCalled()
    })

    it('closes an open editor when the filter changes', async () => {
        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-edit')[0]?.trigger('click')

        expect(wrapper.find('textarea').exists()).toBe(true)

        await wrapper.get('.transcript-pane-filter').setValue('retry')

        expect(wrapper.find('textarea').exists()).toBe(false)
    })

    it('shows an error dialog when the save fails', async () => {
        vi.mocked(ipc.job_transcript_save).mockRejectedValue('api error: locked')

        const { use_dialog } = await import('../../src/composables/use_dialog')
        const dialog = use_dialog()
        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.findAll('.transcript-pane-edit')[0]?.trigger('click')
        await wrapper.get('textarea').setValue('Broken')
        await wrapper.get('textarea').trigger('keydown', { key: 'Enter' })
        await flushPromises()

        expect(dialog.current.value?.title).toBe('The server rejected the request')
        expect(dialog.current.value?.message).toBe('locked')

        dialog.close(true)
    })

    it('does not throw when CSS.highlights is undefined', async () => {
        vi.stubGlobal('CSS', {})

        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.get('.transcript-pane-filter').setValue('retry')
        await flushPromises()

        expect(row_texts(wrapper)).toEqual(['The retry work (v2) is close.'])

        vi.unstubAllGlobals()
    })

    it('registers highlight ranges for filter matches when the registry exists', async () => {
        const registry = new Map<string, unknown>()

        vi.stubGlobal('CSS', { highlights: registry })

        vi.stubGlobal('Highlight', class {
            ranges: Range[]

            constructor(...ranges: Range[]) {
                this.ranges = ranges
            }
        })

        const wrapper = mount_pane()

        await flushPromises()
        await wrapper.get('.transcript-pane-filter').setValue('is')
        await flushPromises()

        const highlight = registry.get('transcript-filter') as { ranges: Range[] } | undefined

        expect(highlight?.ranges.map((range) => range.toString())).toEqual(['is', 'is', 'is'])

        await wrapper.get('.transcript-pane-filter').setValue('')
        await flushPromises()

        expect(registry.has('transcript-filter')).toBe(false)

        vi.unstubAllGlobals()
    })

    it('renders play buttons only when the job has audio', async () => {
        const without = mount_pane()

        await flushPromises()

        expect(without.findAll('.transcript-pane-play')).toHaveLength(0)

        vi.mocked(ipc.job_audio_path_get).mockResolvedValue('/tmp/take.wav')

        const wrapper = mount_pane()

        await flushPromises()

        expect(wrapper.find('audio').exists()).toBe(false)
        expect(wrapper.findAll('.transcript-pane-play')).toHaveLength(3)
    })

    it('attaches the clip for a line to the audio element only when that line is played', async () => {
        const fetch_mock = vi.fn().mockResolvedValue({ ok: true, blob: () => Promise.resolve(new Blob(['wav'])) })
        const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
        const revoke = vi.spyOn(URL, 'revokeObjectURL').mockReturnValue(undefined)

        vi.mocked(ipc.job_audio_path_get).mockResolvedValue('/tmp/take.wav')
        vi.mocked(ipc.job_audio_clip_get).mockResolvedValue('/tmp/clip.wav')
        vi.stubGlobal('fetch', fetch_mock)
        vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:clip')

        const wrapper = mount_pane()

        await flushPromises()

        expect(wrapper.find('audio').exists()).toBe(false)

        await wrapper.get('.transcript-pane-play').trigger('click')
        await flushPromises()

        expect(ipc.job_audio_clip_get).toHaveBeenCalledWith('job-1', expect.any(Number), expect.any(Number))
        expect(fetch_mock).toHaveBeenCalledWith('asset:///tmp/clip.wav')
        expect(wrapper.get('audio').attributes('src')).toBe('blob:clip')
        expect(play).toHaveBeenCalledOnce()
        expect(wrapper.get('.transcript-pane-row').attributes('data-playing')).toBe('true')

        await wrapper.get('audio').trigger('error')

        expect(wrapper.find('audio').exists()).toBe(false)
        expect(wrapper.get('.transcript-pane-row').attributes('data-playing')).toBe('false')
        expect(revoke).toHaveBeenCalledWith('blob:clip')

        vi.unstubAllGlobals()
    })
})
