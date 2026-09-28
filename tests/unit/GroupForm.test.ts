import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import GroupForm from '../../src/components/GroupForm.vue'
import { use_people } from '../../src/composables/use_people'
import { ipc } from '../../src/lib/ipc'
import { group_build, person_build } from './fixtures'
import type { Group } from '../../src/types'
import type { VueWrapper } from '@vue/test-utils'


vi.mock('../../src/lib/ipc')

const JOHN = person_build({ id: 'john', name_first: 'John', name_last: 'Doe', role: 'Developer' })
const JANE = person_build({ id: 'jane', name_first: 'Jane', name_last: 'Doe', role: '' })
const mounted: VueWrapper[] = []
const people_state = use_people()

function mount_form(group: Group | null) {
    const wrapper = mount(GroupForm, { props: { group }, attachTo: document.body })

    mounted.push(wrapper)

    return wrapper
}

function chips(wrapper: VueWrapper) {
    return wrapper.findAll('.tag-chip').map((chip) => chip.text())
}

beforeEach(() => {
    vi.resetAllMocks()
    people_state.people.value = [JOHN, JANE]
    people_state.groups.value = []
    vi.mocked(ipc.people_list).mockResolvedValue([JOHN, JANE])
    vi.mocked(ipc.groups_list).mockResolvedValue([])
})

afterEach(() => {
    for (const wrapper of mounted) wrapper.unmount()

    mounted.length = 0
    document.body.innerHTML = ''
})

describe('GroupForm', () => {
    it('starts empty with save disabled for a new group', () => {
        const wrapper = mount_form(null)

        expect(wrapper.get('input').element.value).toBe('')
        expect(chips(wrapper)).toEqual([])
        expect(wrapper.get('.btn-primary').attributes('disabled')).toBeDefined()
    })

    it('prefills the name and lists known members with their roles', () => {
        const group = group_build({ name: 'Core', person_ids: ['john', 'ghost', 'jane'] })
        const wrapper = mount_form(group)

        expect(wrapper.get('input').element.value).toBe('Core')
        expect(chips(wrapper)).toEqual(['John Doe (Developer)', 'Jane Doe'])
    })

    it('removes a member from its chip', async () => {
        const group = group_build({ person_ids: ['john', 'jane'] })
        const wrapper = mount_form(group)

        await wrapper.get('[aria-label="Remove John Doe"]').trigger('click')

        expect(chips(wrapper)).toEqual(['Jane Doe'])
    })

    it('emits a trimmed group with only the resolvable member ids', async () => {
        const group = group_build({ id: 'g-1', person_ids: ['john', 'ghost'] })
        const wrapper = mount_form(group)

        await wrapper.get('input').setValue('  Core team  ')
        await wrapper.get('.btn-primary').trigger('click')

        expect(wrapper.emitted<Group[]>('save')?.[0]?.[0]).toEqual({
            id: 'g-1',
            name: 'Core team',
            person_ids: ['john'],
        })
    })

    it('mints an id for a new group', async () => {
        const wrapper = mount_form(null)

        await wrapper.get('input').setValue('Fresh')
        await wrapper.get('.btn-primary').trigger('click')

        expect(wrapper.emitted<Group[]>('save')?.[0]?.[0]?.id).toMatch(/^[0-9a-f-]{36}$/)
    })

    it('emits cancel', async () => {
        const wrapper = mount_form(null)

        await wrapper.get('.btn-default').trigger('click')

        expect(wrapper.emitted('cancel')).toHaveLength(1)
    })

    it('opens the picker without groups and adds a ticked person as a member', async () => {
        const wrapper = mount_form(null)

        expect(document.body.querySelector('.people-picker')).toBeNull()

        await wrapper.get('.meeting-details-add').trigger('click')
        await flushPromises()

        const picker = document.body.querySelector('.people-picker')
        const boxes = picker?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]') ?? []

        expect(picker?.querySelector('.app-dialog-title')?.textContent)
            .toBe('Who is in this group?')
        expect(boxes).toHaveLength(2)

        boxes[1]?.click()
        await flushPromises()

        expect(chips(wrapper)).toEqual(['John Doe (Developer)'])

        picker?.querySelector('.btn-primary')?.dispatchEvent(new MouseEvent('click'))
        await flushPromises()

        expect(document.body.querySelector('.people-picker')).toBeNull()
        expect(chips(wrapper)).toEqual(['John Doe (Developer)'])
    })
})
