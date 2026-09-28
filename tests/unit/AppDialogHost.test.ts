import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import AppDialogHost from '../../src/components/AppDialogHost.vue'
import { use_dialog } from '../../src/composables/use_dialog'
import type { VueWrapper } from '@vue/test-utils'


const dialog = use_dialog()

let wrapper: VueWrapper | null = null

function dialog_element() {
    return document.body.querySelector('.app-dialog')
}

function key_press(key: string) {
    const event = new KeyboardEvent('keydown', { key, cancelable: true })

    window.dispatchEvent(event)

    return event
}

beforeEach(() => {
    wrapper = mount(AppDialogHost, { attachTo: document.body })
})

afterEach(() => {
    dialog.close(false)
    wrapper?.unmount()
    wrapper = null
    document.body.style.overflow = ''
})

describe('AppDialogHost', () => {
    it('renders nothing while no dialog is open', () => {
        expect(dialog_element()).toBeNull()
        expect(document.body.style.overflow).toBe('')
    })

    it('renders a confirm dialog with both buttons and locks scrolling', async () => {
        void dialog.confirm('Delete it?', { title: 'Delete recording', kind: 'warning' })
        await wrapper?.vm.$nextTick()

        const element = dialog_element()

        expect(element?.getAttribute('data-kind')).toBe('warning')
        expect(element?.querySelector('.app-dialog-title')?.textContent).toBe('Delete recording')
        expect(element?.querySelector('.app-dialog-message')?.textContent).toBe('Delete it?')
        expect(element?.querySelector('.btn-default')?.textContent.trim()).toBe('Cancel')
        expect(element?.querySelector('.btn-danger')?.textContent.trim()).toBe('OK')
        expect(document.body.style.overflow).toBe('hidden')
    })

    it('renders a message dialog with only a primary button', async () => {
        void dialog.message('Saved.', { kind: 'info' })
        await wrapper?.vm.$nextTick()

        const element = dialog_element()

        expect(element?.getAttribute('data-kind')).toBe('info')
        expect(element?.querySelector('.btn-default')).toBeNull()
        expect(element?.querySelector('.btn-primary')?.textContent.trim()).toBe('OK')
    })

    it('resolves true from the ok button and unlocks scrolling', async () => {
        const pending = dialog.confirm('Go?')

        await wrapper?.vm.$nextTick()

        dialog_element()?.querySelector('.btn-danger')?.dispatchEvent(
            new MouseEvent('click', { bubbles: true }),
        )

        expect(await pending).toBe(true)

        await wrapper?.vm.$nextTick()

        expect(dialog_element()).toBeNull()
        expect(document.body.style.overflow).toBe('')
    })

    it('resolves false from cancel, the close button and the backdrop', async () => {
        const first = dialog.confirm('One')

        await wrapper?.vm.$nextTick()

        dialog_element()?.querySelector('.btn-default')?.dispatchEvent(
            new MouseEvent('click', { bubbles: true }),
        )

        expect(await first).toBe(false)

        const second = dialog.confirm('Two')

        await wrapper?.vm.$nextTick()

        dialog_element()?.querySelector('.app-dialog-close')?.dispatchEvent(
            new MouseEvent('click', { bubbles: true }),
        )

        expect(await second).toBe(false)

        const third = dialog.confirm('Three')

        await wrapper?.vm.$nextTick()

        document.body.querySelector('.app-dialog-backdrop')?.dispatchEvent(
            new MouseEvent('click', { bubbles: true }),
        )

        expect(await third).toBe(false)
    })

    it('does not close when the dialog body itself is clicked', async () => {
        void dialog.confirm('Stay')

        await wrapper?.vm.$nextTick()
        dialog_element()?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
        await wrapper?.vm.$nextTick()

        expect(dialog_element()).not.toBeNull()
    })

    it('answers escape with false and enter with true', async () => {
        const first = dialog.confirm('Escape me')

        await wrapper?.vm.$nextTick()

        const escape = key_press('Escape')

        expect(await first).toBe(false)
        expect(escape.defaultPrevented).toBe(true)

        const second = dialog.confirm('Enter me')

        await wrapper?.vm.$nextTick()

        const enter = key_press('Enter')

        expect(await second).toBe(true)
        expect(enter.defaultPrevented).toBe(true)
    })

    it('ignores keys while no dialog is open', () => {
        expect(key_press('Escape').defaultPrevented).toBe(false)
        expect(key_press('Enter').defaultPrevented).toBe(false)
    })

    it('stops handling keys after unmount', async () => {
        wrapper?.unmount()
        wrapper = null

        const pending = dialog.confirm('Orphan')

        expect(key_press('Enter').defaultPrevented).toBe(false)

        dialog.close(false)

        expect(await pending).toBe(false)
    })
})
