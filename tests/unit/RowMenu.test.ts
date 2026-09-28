import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import RowMenu from '../../src/components/RowMenu.vue'
import type { RowMenuItem } from '../../src/components/RowMenu.vue'
import type { VueWrapper } from '@vue/test-utils'


const ITEMS: RowMenuItem[] = [
    { id: 'rename', label: 'Rename', icon: 'edit' },
    { id: 'delete', label: 'Delete', icon: 'delete', danger: true },
]

let wrapper: VueWrapper | null = null

function mount_menu() {
    wrapper = mount(RowMenu, {
        props: { label: 'Weekly sync', items: ITEMS },
        attachTo: document.body,
    })

    return wrapper
}

function popover() {
    return document.body.querySelector('.menu-popover')
}

async function menu_open(menu: VueWrapper) {
    await menu.get('.menu-icon-btn').trigger('click')
}

afterEach(() => {
    wrapper?.unmount()
    wrapper = null
})

describe('RowMenu', () => {
    it('labels the trigger and starts closed', () => {
        const menu = mount_menu()
        const trigger = menu.get('.menu-icon-btn')

        expect(trigger.attributes('aria-label')).toBe('Options for Weekly sync')
        expect(trigger.attributes('aria-haspopup')).toBe('menu')
        expect(trigger.attributes('aria-expanded')).toBe('false')
        expect(popover()).toBeNull()
    })

    it('opens a menu in the body listing every item with danger styling', async () => {
        const menu = mount_menu()

        await menu_open(menu)

        const items = popover()?.querySelectorAll('[role="menuitem"]') ?? []

        expect(menu.get('.menu-icon-btn').attributes('aria-expanded')).toBe('true')
        expect(popover()?.getAttribute('role')).toBe('menu')
        expect(items).toHaveLength(2)
        expect(items[0]?.textContent.trim()).toBe('Rename')
        expect(items[0]?.classList.contains('menu-item-danger')).toBe(false)
        expect(items[1]?.textContent.trim()).toBe('Delete')
        expect(items[1]?.classList.contains('menu-item-danger')).toBe(true)
    })

    it('draws the icon paths for each item', async () => {
        const menu = mount_menu()

        await menu_open(menu)

        const paths = popover()?.querySelectorAll('[role="menuitem"]')[1]?.querySelectorAll('path')

        expect(paths).toHaveLength(5)
    })

    it('emits the item id and closes on selection', async () => {
        const menu = mount_menu()

        await menu_open(menu)

        const delete_item = popover()?.querySelectorAll('[role="menuitem"]')[1]

        delete_item?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
        await menu.vm.$nextTick()

        expect(menu.emitted('select')).toEqual([['delete']])
        expect(popover()).toBeNull()
    })

    it('closes when the trigger is clicked again', async () => {
        const menu = mount_menu()

        await menu_open(menu)
        await menu_open(menu)

        expect(popover()).toBeNull()
    })

    it('closes on escape, outside mousedown, scroll and resize', async () => {
        const menu = mount_menu()

        await menu_open(menu)
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
        await menu.vm.$nextTick()

        expect(popover()).toBeNull()

        await menu_open(menu)
        document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
        await menu.vm.$nextTick()

        expect(popover()).toBeNull()

        await menu_open(menu)
        document.dispatchEvent(new Event('scroll'))
        await menu.vm.$nextTick()

        expect(popover()).toBeNull()

        await menu_open(menu)
        window.dispatchEvent(new Event('resize'))
        await menu.vm.$nextTick()

        expect(popover()).toBeNull()
    })

    it('stays open on a mousedown inside the popover or the trigger', async () => {
        const menu = mount_menu()

        await menu_open(menu)
        popover()?.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))

        menu.get('.menu-icon-btn').element.dispatchEvent(
            new MouseEvent('mousedown', { bubbles: true }),
        )

        await menu.vm.$nextTick()

        expect(popover()).not.toBeNull()
    })

    it('ignores other keys', async () => {
        const menu = mount_menu()

        await menu_open(menu)
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }))
        await menu.vm.$nextTick()

        expect(popover()).not.toBeNull()
    })
})
