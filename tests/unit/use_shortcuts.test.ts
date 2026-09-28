import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { KeepAlive, defineComponent, h, nextTick, ref } from 'vue'
import { use_shortcuts } from '../../src/composables/use_shortcuts'
import type { VueWrapper } from '@vue/test-utils'


type Handler = (event: KeyboardEvent) => void

const mounted: VueWrapper[] = []

function shortcut_component(shortcuts: Record<string, Handler>) {
    return defineComponent({
        setup() {
            use_shortcuts(shortcuts)

            return () => h('div')
        },
    })
}

function mount_shortcuts(shortcuts: Record<string, Handler>) {
    const wrapper = mount(shortcut_component(shortcuts), { attachTo: document.body })

    mounted.push(wrapper)

    return wrapper
}

function key_press(
    key: string,
    modifiers: Partial<Pick<KeyboardEvent, 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'>> = {},
    target: EventTarget = window,
) {
    const init = { key, bubbles: true, cancelable: true, ...modifiers }
    const event = new KeyboardEvent('keydown', init)

    target.dispatchEvent(event)

    return event
}

function element_attach<T extends HTMLElement>(element: T): T {
    document.body.appendChild(element)

    return element
}

afterEach(() => {
    for (const wrapper of mounted) wrapper.unmount()

    mounted.length = 0
    document.body.innerHTML = ''
})

describe('use_shortcuts', () => {
    it('runs the handler for a plain key and prevents the default', () => {
        const handler = vi.fn()

        mount_shortcuts({ s: handler })

        const event = key_press('S')

        expect(handler).toHaveBeenCalledWith(event)
        expect(event.defaultPrevented).toBe(true)
    })

    it('builds the combo as ctrl, shift, alt then the key', () => {
        const handler = vi.fn()

        mount_shortcuts({ 'ctrl+shift+alt+k': handler })
        key_press('k', { ctrlKey: true, shiftKey: true, altKey: true })

        expect(handler).toHaveBeenCalledTimes(1)
    })

    it('treats the meta key as ctrl', () => {
        const handler = vi.fn()

        mount_shortcuts({ 'ctrl+n': handler })
        key_press('n', { metaKey: true })

        expect(handler).toHaveBeenCalledTimes(1)
    })

    it('names the space bar space', () => {
        const handler = vi.fn()

        mount_shortcuts({ space: handler })
        key_press(' ')

        expect(handler).toHaveBeenCalledTimes(1)
    })

    it('leaves an unregistered key untouched', () => {
        mount_shortcuts({ s: vi.fn() })

        const event = key_press('x')

        expect(event.defaultPrevented).toBe(false)
    })

    it('ignores plain keys typed into text inputs, textareas and editable elements', () => {
        const handler = vi.fn()

        mount_shortcuts({ s: handler })

        const input = element_attach(document.createElement('input'))
        const textarea = element_attach(document.createElement('textarea'))
        const editable = element_attach(document.createElement('div'))

        editable.contentEditable = 'true'

        key_press('s', {}, input)
        key_press('s', {}, textarea)
        key_press('s', {}, editable)

        expect(handler).not.toHaveBeenCalled()
    })

    it('still fires a ctrl combo from inside a text input', () => {
        const handler = vi.fn()

        mount_shortcuts({ 'ctrl+s': handler })

        const input = element_attach(document.createElement('input'))

        key_press('s', { ctrlKey: true }, input)

        expect(handler).toHaveBeenCalledTimes(1)
    })

    it('fires a plain key from a non-text input such as a checkbox', () => {
        const handler = vi.fn()

        mount_shortcuts({ s: handler })

        const checkbox = element_attach(document.createElement('input'))

        checkbox.type = 'checkbox'
        key_press('s', {}, checkbox)

        expect(handler).toHaveBeenCalledTimes(1)
    })

    it('gives the most recently mounted registration priority and restores on unmount', () => {
        const first = vi.fn()
        const second = vi.fn()

        mount_shortcuts({ s: first })

        const wrapper_second = mount_shortcuts({ s: second })

        key_press('s')

        expect(first).not.toHaveBeenCalled()
        expect(second).toHaveBeenCalledTimes(1)

        wrapper_second.unmount()
        mounted.pop()
        key_press('s')

        expect(first).toHaveBeenCalledTimes(1)
        expect(second).toHaveBeenCalledTimes(1)
    })

    it('unregisters while deactivated in KeepAlive and re-registers on activation', async () => {
        const handler = vi.fn()
        const shown = ref(true)
        const Child = shortcut_component({ s: handler })

        const host = {
            setup() {
                return () => h(KeepAlive, null, { default: () => (shown.value ? h(Child) : null) })
            },
        }

        mounted.push(mount(host, { attachTo: document.body }))
        key_press('s')

        expect(handler).toHaveBeenCalledTimes(1)

        shown.value = false
        await nextTick()
        key_press('s')

        expect(handler).toHaveBeenCalledTimes(1)

        shown.value = true
        await nextTick()
        key_press('s')

        expect(handler).toHaveBeenCalledTimes(2)
    })
})
