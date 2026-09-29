import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import RecordingTile from '../../src/components/RecordingTile.vue'
import { ipc } from '../../src/lib/ipc'
import { job_listing_build } from './fixtures'
import type { RowMenuItem } from '../../src/components/RowMenu.vue'
import type { JobListing } from '../../src/types'
import type { VueWrapper } from '@vue/test-utils'


vi.mock('../../src/lib/ipc')

const MENU_ITEMS: RowMenuItem[] = [{ id: 'rename', label: 'Rename', icon: 'edit' }]
const mounted: VueWrapper[] = []

function mount_tile(job: JobListing, people: string | null = null, snippet: string | null = null) {
    const wrapper = mount(RecordingTile, {
        props: {
            job,
            title: 'Weekly sync',
            when: 'Today, 9:00 AM',
            people,
            snippet,
            menu_items: MENU_ITEMS,
        },
        attachTo: document.body,
    })

    mounted.push(wrapper)

    return wrapper
}

beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(ipc.job_waveform_get).mockResolvedValue(null)
})

afterEach(() => {
    for (const wrapper of mounted) wrapper.unmount()

    mounted.length = 0
    document.body.innerHTML = ''
})

describe('RecordingTile', () => {
    it('shows the title, date, people and status', async () => {
        const job = job_listing_build({ has_transcript: true })
        const wrapper = mount_tile(job, 'Jane Doe, John Doe')

        await flushPromises()

        expect(wrapper.get('.recording-tile-title').text()).toBe('Weekly sync')
        expect(wrapper.get('.recording-tile-people').text()).toBe('Jane Doe, John Doe')
        expect(wrapper.get('.recording-tile-foot span').text()).toBe('Today, 9:00 AM')
        expect(wrapper.get('.recording-tile-status').text()).toBe('Transcribed')
        expect(wrapper.get('.recording-tile-status').attributes('data-state')).toBe('transcript')
    })

    it('prefers the search snippet over the people text', async () => {
        const wrapper = mount_tile(job_listing_build(), 'Jane Doe', 'In notes: retry policy')

        await flushPromises()

        expect(wrapper.get('.recording-tile-people').text()).toBe('In notes: retry policy')
    })

    it('falls back to a no participants label', async () => {
        const wrapper = mount_tile(job_listing_build())

        await flushPromises()

        expect(wrapper.get('.recording-tile-people').text()).toBe('No participants')
    })

    it('shows the duration only when known', async () => {
        const without = mount_tile(job_listing_build())
        const with_duration = mount_tile(job_listing_build({ duration_seconds: 3725 }))

        await flushPromises()

        expect(without.find('.recording-tile-duration').exists()).toBe(false)
        expect(with_duration.get('.recording-tile-duration').text()).toBe('1:02:05')
    })

    it('draws one bar per waveform peak with a minimum height', async () => {
        vi.mocked(ipc.job_waveform_get).mockResolvedValue({
            peaks: [0, 0.5, 1],
            duration_seconds: 30,
        })

        const wrapper = mount_tile(job_listing_build({ id: 'job-wave' }))

        await flushPromises()

        const bars = wrapper.findAll('rect')

        expect(ipc.job_waveform_get).toHaveBeenCalledWith('job-wave', 110)
        expect(wrapper.get('svg').attributes('viewBox')).toBe('0 0 9 100')
        expect(bars.map((bar) => bar.attributes('height'))).toEqual(['4', '50', '100'])
        expect(bars.map((bar) => bar.attributes('x'))).toEqual(['0', '3', '6'])
        expect(bars[1]?.attributes('y')).toBe('25')
    })

    it('draws a text glyph instead of a wave when the job has no audio', async () => {
        const wrapper = mount_tile(job_listing_build())

        await flushPromises()

        const glyph = wrapper.get('.recording-tile-glyph')
        const words = glyph.findAll('rect')
        const rows = new Set(words.map((word) => word.attributes('y')))

        expect(glyph.attributes('viewBox')).toBe('0 0 100 100')
        expect(words.length).toBeGreaterThan(5)
        expect(rows.size).toBe(5)
        expect(words.every((word) => word.attributes('height') === '8')).toBe(true)
    })

    it('draws the same glyph for the same job and a different one for another', async () => {
        const first = mount_tile(job_listing_build({ id: 'job-a' }))
        const same = mount_tile(job_listing_build({ id: 'job-a' }))
        const other = mount_tile(job_listing_build({ id: 'job-b' }))

        await flushPromises()

        const shape = (wrapper: VueWrapper) =>
            wrapper.findAll('rect').map((word) => word.attributes('width')).join(',')

        expect(shape(first)).toBe(shape(same))
        expect(shape(first)).not.toBe(shape(other))
    })

    it('warns and keeps rendering when the waveform fails', async () => {
        const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)

        vi.mocked(ipc.job_waveform_get).mockRejectedValue(new Error('corrupt'))

        const wrapper = mount_tile(job_listing_build({ id: 'job-fail' }))

        await flushPromises()

        expect(warn).toHaveBeenCalledWith(
            '[recordings] waveform unavailable',
            'job-fail',
            expect.any(Error),
        )

        expect(wrapper.get('.recording-tile-title').text()).toBe('Weekly sync')

        warn.mockRestore()
    })

    it('emits open on click and on enter', async () => {
        const wrapper = mount_tile(job_listing_build())

        await wrapper.get('article').trigger('click')
        await wrapper.get('article').trigger('keydown', { key: 'Enter' })

        expect(wrapper.emitted('open')).toHaveLength(2)
    })

    it('emits favourite from the star without opening', async () => {
        const wrapper = mount_tile(job_listing_build({ favourite: true }))

        expect(wrapper.get('.recording-tile-star').attributes('aria-pressed')).toBe('true')

        await wrapper.get('.recording-tile-star').trigger('click')

        expect(wrapper.emitted('favourite')).toHaveLength(1)
        expect(wrapper.emitted('open')).toBeUndefined()
    })

    it('forwards a menu selection with its id', async () => {
        const wrapper = mount_tile(job_listing_build())

        await wrapper.get('.menu-icon-btn').trigger('click')

        document.body.querySelector('[role="menuitem"]')?.dispatchEvent(
            new MouseEvent('click', { bubbles: true }),
        )

        await wrapper.vm.$nextTick()

        expect(wrapper.emitted('menu')).toEqual([['rename']])
        expect(wrapper.emitted('open')).toBeUndefined()
    })
})
