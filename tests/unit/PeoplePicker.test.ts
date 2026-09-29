import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import PeoplePicker from '../../src/components/PeoplePicker.vue'
import { use_dialog } from '../../src/composables/use_dialog'
import { use_people } from '../../src/composables/use_people'
import { ipc } from '../../src/lib/ipc'
import { group_build, person_build } from './fixtures'
import type { Group, Person } from '../../src/types'
import type { VueWrapper } from '@vue/test-utils'


vi.mock('../../src/lib/ipc')

const JOHN = person_build({ id: 'john', name_first: 'John', name_last: 'Doe', role: 'Developer' })
const JANE = person_build({ id: 'jane', name_first: 'Jane', name_last: 'Doe', role: 'Owner' })
const SAM = person_build({ id: 'sam', name_first: 'Sam', name_last: 'Adams', role: '' })
const TEAM = group_build({ id: 'team', name: 'Product team', person_ids: ['john', 'jane'] })
const EMPTY = group_build({ id: 'empty', name: 'Alumni', person_ids: ['ghost'] })
const mounted: VueWrapper[] = []
const people_state = use_people()
const dialog = use_dialog()

async function mount_picker(
    ids_selected: string[] = [],
    people: Person[] = [JOHN, JANE, SAM],
    groups: Group[] = [TEAM, EMPTY],
    groups_hidden = false,
) {
    vi.mocked(ipc.people_list).mockResolvedValue(people)
    vi.mocked(ipc.groups_list).mockResolvedValue(groups)

    const wrapper = mount(PeoplePicker, {
        props: { ids_selected, title: 'Who was there?', hint: 'Tick everyone.', groups_hidden },
        attachTo: document.body,
    })

    mounted.push(wrapper)
    await flushPromises()

    return wrapper
}

function root() {
    return document.body.querySelector('.people-picker')
}

function rows() {
    return Array.from(root()?.querySelectorAll('.people-picker-row') ?? [])
}

function row_texts() {
    return rows().map((row) =>
        Array.from(row.querySelectorAll('span'))
            .filter((span) => span.children.length === 0)
            .map((span) => span.textContent.trim())
            .join(' '),
    )
}

function click(target: Element | null | undefined) {
    target?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
}

function checkbox_at(index: number) {
    return rows()[index]?.querySelector<HTMLInputElement>('input[type="checkbox"]') ?? null
}

function emitted_ids(wrapper: Awaited<ReturnType<typeof mount_picker>>) {
    return wrapper.emitted<string[][]>('update:ids_selected')?.map((call) => call[0])
}

beforeEach(() => {
    vi.resetAllMocks()
    people_state.people.value = []
    people_state.groups.value = []
    people_state.error_message.value = null
})

afterEach(() => {
    for (const wrapper of mounted) wrapper.unmount()

    mounted.length = 0
    dialog.close(false)
    document.body.innerHTML = ''
})

describe('PeoplePicker', () => {
    it('loads people and groups on mount and lists non-empty groups first', async () => {
        await mount_picker()

        expect(ipc.people_list).toHaveBeenCalledTimes(1)
        expect(ipc.groups_list).toHaveBeenCalledTimes(1)
        expect(root()?.querySelector('.app-dialog-title')?.textContent).toBe('Who was there?')
        expect(root()?.querySelector('.app-dialog-message')?.textContent).toBe('Tick everyone.')

        expect(row_texts()).toEqual([
            'Product team 2 people',
            'Sam Adams',
            'Jane Doe (Owner)',
            'John Doe (Developer)',
        ])

        expect(root()?.querySelector('.people-picker-divider')).not.toBeNull()
    })

    it('hides groups when asked', async () => {
        await mount_picker([], [JOHN], [TEAM], true)

        expect(row_texts()).toEqual(['John Doe (Developer)'])
        expect(root()?.querySelector('.people-picker-divider')).toBeNull()
    })

    it('reflects the selected ids in the checkboxes', async () => {
        await mount_picker(['jane'])

        expect(checkbox_at(0)?.checked).toBe(false)
        expect(checkbox_at(2)?.checked).toBe(true)
        expect(checkbox_at(3)?.checked).toBe(false)
    })

    it('ticks a group when every member is selected', async () => {
        await mount_picker(['john', 'jane'])

        expect(checkbox_at(0)?.checked).toBe(true)
    })

    it('emits the id added or removed when a person is toggled', async () => {
        const wrapper = await mount_picker(['sam'])

        checkbox_at(3)?.click()
        await flushPromises()

        expect(emitted_ids(wrapper)?.[0]).toEqual(['sam', 'john'])

        checkbox_at(1)?.click()
        await flushPromises()

        expect(emitted_ids(wrapper)?.[1]).toEqual([])
    })

    it('adds the missing members of a group and removes them all when ticked again', async () => {
        const wrapper = await mount_picker(['john', 'sam'])

        checkbox_at(0)?.click()
        await flushPromises()

        expect(emitted_ids(wrapper)?.[0]).toEqual(['john', 'sam', 'jane'])

        await wrapper.setProps({ ids_selected: ['john', 'sam', 'jane'] })
        checkbox_at(0)?.click()
        await flushPromises()

        expect(emitted_ids(wrapper)?.[1]).toEqual(['sam'])
    })

    it('filters people by name or role and groups by name', async () => {
        await mount_picker()

        const filter = root()?.querySelector<HTMLInputElement>('input[aria-label="Find a person"]')

        if (!filter) throw new Error('filter input missing')

        filter.value = 'owner'
        filter.dispatchEvent(new Event('input'))
        await flushPromises()

        expect(row_texts()).toEqual(['Jane Doe (Owner)'])

        filter.value = 'product'
        filter.dispatchEvent(new Event('input'))
        await flushPromises()

        expect(row_texts()).toEqual(['Product team 2 people'])

        filter.value = 'zzz'
        filter.dispatchEvent(new Event('input'))
        await flushPromises()

        expect(rows()).toHaveLength(0)
        expect(root()?.querySelector('.meta')?.textContent).toBe('Nobody matches that name.')
    })

    it('explains when there are no people at all', async () => {
        await mount_picker([], [], [])

        expect(root()?.querySelector('.meta')?.textContent).toBe(
            'No people yet. Add the first one below.',
        )
    })

    it('emits close from done, the close button, the backdrop and escape', async () => {
        const wrapper = await mount_picker()

        click(root()?.querySelector('.btn-primary'))
        click(root()?.querySelector('.app-dialog-close'))
        click(document.body.querySelector('.app-dialog-backdrop'))
        click(root())

        document.body.querySelector('.app-dialog-backdrop')?.dispatchEvent(
            new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
        )

        expect(wrapper.emitted('close')).toHaveLength(4)
    })

    it('creates a person, selects them and returns to the list', async () => {
        const wrapper = await mount_picker(['sam'])

        vi.mocked(ipc.person_save).mockResolvedValue(undefined)
        click(root()?.querySelector('.btn-default'))
        await flushPromises()

        const first_name = root()?.querySelector<HTMLInputElement>('input[placeholder="e.g. John"]')

        if (!first_name) throw new Error('person form missing')

        expect(root()?.querySelector('input[aria-label="Find a person"]')).toBeNull()

        first_name.value = 'Ada'
        first_name.dispatchEvent(new Event('input'))
        await flushPromises()

        vi.mocked(ipc.people_list).mockResolvedValue([
            JOHN,
            JANE,
            SAM,
            person_build({ id: 'ada', name_first: 'Ada', name_last: '' }),
        ])

        click(root()?.querySelector('.btn-primary'))
        await flushPromises()

        const saved = vi.mocked(ipc.person_save).mock.calls[0]?.[0]

        expect(saved?.name_first).toBe('Ada')
        expect(emitted_ids(wrapper)?.[0]).toEqual(['sam', saved?.id])
        expect(root()?.querySelector('input[aria-label="Find a person"]')).not.toBeNull()
        expect(dialog.current.value).toBeNull()
    })

    it('shows an error dialog and stays on the form when saving fails', async () => {
        await mount_picker()

        vi.mocked(ipc.person_save).mockRejectedValue(new Error('duplicate'))
        click(root()?.querySelector('.btn-default'))
        await flushPromises()

        const first_name = root()?.querySelector<HTMLInputElement>('input[placeholder="e.g. John"]')

        if (!first_name) throw new Error('person form missing')

        first_name.value = 'Ada'
        first_name.dispatchEvent(new Event('input'))
        await flushPromises()
        click(root()?.querySelector('.btn-primary'))
        await flushPromises()

        expect(dialog.current.value?.message).toBe('Error: duplicate')
        expect(root()?.querySelector('input[placeholder="e.g. John"]')).not.toBeNull()
    })
})
