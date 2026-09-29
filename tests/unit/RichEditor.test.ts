import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TextSelection } from '@tiptap/pm/state'
import RichEditor from '../../src/components/RichEditor.vue'
import { ipc } from '../../src/lib/ipc'
import type { Editor } from '@tiptap/core'
import type { AIRewriteArgs, AIRewriteHandler } from '../../src/types'
import type { VueWrapper } from '@vue/test-utils'


vi.mock('../../src/lib/ipc')

const MARKDOWN = 'First paragraph\n\n- item one\n- item two'
const mounted: VueWrapper[] = []

class ResizeObserverStub {
    observe() {
        return undefined
    }

    disconnect() {
        return undefined
    }
}

function rewrite_handler(reply: string) {
    return vi.fn((args: AIRewriteArgs) => {
        args.on_chunk(reply)

        return Promise.resolve(reply)
    })
}

async function mount_editor(modelValue = MARKDOWN, ai_rewrite?: AIRewriteHandler) {
    const wrapper = mount(RichEditor, {
        props: ai_rewrite ? { modelValue, ai_rewrite } : { modelValue },
        attachTo: document.body,
    })

    mounted.push(wrapper)
    await flushPromises()

    return wrapper
}

function editor_of(wrapper: VueWrapper): Editor {
    const element = wrapper.get('.ProseMirror').element as HTMLElement & { editor?: Editor }

    if (!element.editor) throw new Error('tiptap did not attach the editor to its view')

    return element.editor
}

function select(editor: Editor, from: number, to: number) {
    const transaction = editor.state.tr.setSelection(
        TextSelection.create(editor.state.doc, from, to),
    )

    editor.view.dispatch(transaction)
}

function collapse(editor: Editor, position: number) {
    editor.view.dispatch(
        editor.state.tr
            .setSelection(TextSelection.create(editor.state.doc, position))
            .setMeta('pointer', true),
    )
}

function ai_meta(wrapper: VueWrapper) {
    const metas = wrapper.findAll('.rich-editor-ai-meta')

    return metas[metas.length - 1]?.text()
}

function model_emitted_last(wrapper: VueWrapper) {
    const calls = wrapper.emitted<string[]>('update:modelValue') ?? []

    return calls[calls.length - 1]
}

function text_range(editor: Editor, first: string, last: string) {
    let from = -1
    let to = -1

    editor.state.doc.descendants((node, pos) => {
        if (node.text === first) from = pos
        if (node.text === last) to = pos + node.nodeSize
    })

    if (from < 0 || to < 0) throw new Error(`text nodes ${first} and ${last} not found`)

    return { from, to }
}

function ai_button(wrapper: VueWrapper) {
    return wrapper.get('.rich-tb-btn-ai')
}

function quick_action(wrapper: VueWrapper, label: string) {
    const button = wrapper.findAll('.rich-editor-ai-actions button').find(
        (candidate) => candidate.text() === label,
    )

    if (!button) throw new Error(`no quick action labelled ${label}`)

    return button
}

beforeEach(() => {
    vi.resetAllMocks()
    vi.stubGlobal('ResizeObserver', ResizeObserverStub)

    vi.mocked(ipc.markdown_render_html).mockImplementation(
        (markdown) => Promise.resolve(`<p>${markdown}</p>`),
    )
})

afterEach(() => {
    for (const wrapper of mounted) wrapper.unmount()

    mounted.length = 0
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
    document.body.innerHTML = ''
})

describe('RichEditor mounting', () => {
    it('renders the markdown as a document and reports word and character counts', async () => {
        const wrapper = await mount_editor()
        const editor = editor_of(wrapper)

        expect(editor.getHTML()).toBe(
            '<p>First paragraph</p><ul class="tight" data-tight="true">'
                + '<li><p>item one</p></li><li><p>item two</p></li></ul>',
        )

        expect(wrapper.get('.rich-editor-stats').text()).toContain('8 words')
        expect(wrapper.get('.rich-editor-stats').text()).toContain(`${MARKDOWN.length} characters`)
    })

    it('emits markdown when the document changes', async () => {
        const wrapper = await mount_editor()
        const editor = editor_of(wrapper)

        editor.commands.insertContentAt(1, 'Very ')
        await flushPromises()

        expect(model_emitted_last(wrapper)).toEqual([
            'Very First paragraph\n\n- item one\n- item two',
        ])
    })
})

describe('SelectionGuard', () => {
    let now = 0

    beforeEach(() => {
        now = 1_000
        vi.spyOn(performance, 'now').mockImplementation(() => now)
    })

    function arm(editor: Editor) {
        select(editor, 1, 16)
        editor.view.dispatch(editor.state.tr.insert(16, editor.schema.text('!')))

        expect(editor.state.selection.from).toBe(1)
        expect(editor.state.selection.to).toBe(17)
    }

    it('rejects a bare collapse within the window after a doc change with a range', async () => {
        const editor = editor_of(await mount_editor())

        arm(editor)
        now += 100

        editor.view.dispatch(
            editor.state.tr.setSelection(TextSelection.create(editor.state.doc, 3)),
        )

        expect(editor.state.selection.empty).toBe(false)
        expect(editor.state.selection.from).toBe(1)
        expect(editor.state.selection.to).toBe(17)
    })

    it('accepts a collapse carrying the pointer meta', async () => {
        const editor = editor_of(await mount_editor())

        arm(editor)
        now += 100

        editor.view.dispatch(
            editor.state.tr
                .setSelection(TextSelection.create(editor.state.doc, 3))
                .setMeta('pointer', true),
        )

        expect(editor.state.selection.empty).toBe(true)
        expect(editor.state.selection.from).toBe(3)
    })

    it('accepts a collapse carrying the uiEvent meta or a scrollIntoView marker', async () => {
        const editor = editor_of(await mount_editor())

        arm(editor)
        now += 50

        editor.view.dispatch(
            editor.state.tr
                .setSelection(TextSelection.create(editor.state.doc, 4))
                .setMeta('uiEvent', 'drop'),
        )

        expect(editor.state.selection.from).toBe(4)

        arm(editor)
        now += 50

        editor.view.dispatch(
            editor.state.tr
                .setSelection(TextSelection.create(editor.state.doc, 5))
                .scrollIntoView(),
        )

        expect(editor.state.selection.from).toBe(5)
    })

    it('accepts a collapse that also changes the document', async () => {
        const editor = editor_of(await mount_editor())

        arm(editor)
        now += 100

        editor.view.dispatch(
            editor.state.tr
                .setSelection(TextSelection.create(editor.state.doc, 2))
                .insertText('Z', 2, 2),
        )

        expect(editor.state.selection.empty).toBe(true)
        expect(editor.state.doc.textBetween(1, 4)).toBe('FZi')
    })

    it('accepts a bare collapse once the window has passed', async () => {
        const editor = editor_of(await mount_editor())

        arm(editor)
        now += 251

        editor.view.dispatch(
            editor.state.tr.setSelection(TextSelection.create(editor.state.doc, 3)),
        )

        expect(editor.state.selection.empty).toBe(true)
    })

    it('does not arm on a doc change made with a collapsed selection', async () => {
        const editor = editor_of(await mount_editor())

        select(editor, 2, 2)
        editor.view.dispatch(editor.state.tr.insertText('!', 2, 2))
        select(editor, 1, 10)
        now += 10

        editor.view.dispatch(
            editor.state.tr.setSelection(TextSelection.create(editor.state.doc, 3)),
        )

        expect(editor.state.selection.empty).toBe(true)
    })

    it('lets a range grow or move while armed', async () => {
        const editor = editor_of(await mount_editor())

        arm(editor)
        now += 10
        select(editor, 2, 8)

        expect(editor.state.selection.from).toBe(2)
        expect(editor.state.selection.to).toBe(8)
    })
})

describe('RichEditor AI rewrite', () => {
    beforeEach(() => {
        vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    })

    it('follows the selection while the panel is open', async () => {
        const rewrite = rewrite_handler('Rewritten paragraph')
        const wrapper = await mount_editor(MARKDOWN, rewrite)
        const editor = editor_of(wrapper)

        select(editor, 1, 6)
        await ai_button(wrapper).trigger('click')

        expect(ai_meta(wrapper)).toBe('5 characters selected')

        collapse(editor, 20)
        await flushPromises()

        expect(editor.state.selection.empty).toBe(true)
        expect(ai_meta(wrapper)).toBe('Whole document')

        select(editor, 1, 16)
        await flushPromises()

        expect(ai_meta(wrapper)).toBe('15 characters selected')

        await quick_action(wrapper, 'Rewrite').trigger('click')
        await flushPromises()

        expect(rewrite).toHaveBeenCalledTimes(1)

        const args = rewrite.mock.calls[0]?.[0]

        expect(args?.text).toBe('First paragraph')
        expect(args?.whole_document).toBe(false)

        expect(args?.instruction).toBe(
            'Rewrite this more clearly and concisely. Keep the same meaning.',
        )

        expect(ipc.markdown_render_html).toHaveBeenCalledWith('Rewritten paragraph')

        expect(model_emitted_last(wrapper)).toEqual([
            'Rewritten paragraph\n\n- item one\n- item two',
        ])
    })

    it('sends the whole document flagged as such when nothing is selected', async () => {
        const rewrite = rewrite_handler('All new text')
        const wrapper = await mount_editor(MARKDOWN, rewrite)

        await ai_button(wrapper).trigger('click')

        expect(ai_meta(wrapper)).toBe('Whole document')

        await wrapper.get('.rich-editor-ai-input').setValue('Make it formal')
        await wrapper.get('.rich-editor-ai-custom .btn-primary').trigger('click')
        await flushPromises()

        const args = rewrite.mock.calls[0]?.[0]

        expect(args?.text).toBe(MARKDOWN)
        expect(args?.whole_document).toBe(true)
        expect(args?.instruction).toBe('Make it formal')
        expect(model_emitted_last(wrapper)).toEqual(['All new text'])
    })

    it('serialises a selection inside a bullet list as dash lines', async () => {
        const rewrite = rewrite_handler('- merged item')
        const wrapper = await mount_editor(MARKDOWN, rewrite)
        const editor = editor_of(wrapper)
        const { from, to } = text_range(editor, 'item one', 'item two')

        select(editor, from, to)

        expect(editor.state.doc.textBetween(from, to, '\n')).toBe('item one\nitem two')

        await ai_button(wrapper).trigger('click')
        await quick_action(wrapper, 'Shorten').trigger('click')
        await flushPromises()

        expect(rewrite.mock.calls[0]?.[0]?.text).toBe('- item one\n- item two')
        expect(rewrite.mock.calls[0]?.[0]?.whole_document).toBe(false)
    })

    it('refuses an empty custom instruction', async () => {
        const rewrite = rewrite_handler('x')
        const wrapper = await mount_editor(MARKDOWN, rewrite)

        await ai_button(wrapper).trigger('click')
        await wrapper.get('.rich-editor-ai-input').setValue('   ')
        await wrapper.get('.rich-editor-ai-input').trigger('keydown', { key: 'Enter' })
        await flushPromises()

        expect(rewrite).not.toHaveBeenCalled()
        expect(wrapper.get('.rich-editor-ai-error').text()).toBe('Enter an instruction.')
    })

    it('falls back to the whole document when the content is replaced', async () => {
        const rewrite = rewrite_handler('x')
        const wrapper = await mount_editor(MARKDOWN, rewrite)
        const editor = editor_of(wrapper)
        const { from, to } = text_range(editor, 'item two', 'item two')

        select(editor, from, to)
        await ai_button(wrapper).trigger('click')

        expect(ai_meta(wrapper)).toBe('8 characters selected')

        editor.commands.setContent('Tiny', { emitUpdate: false })
        await flushPromises()

        expect(ai_meta(wrapper)).toBe('Whole document')

        await quick_action(wrapper, 'Expand').trigger('click')
        await flushPromises()

        expect(rewrite.mock.calls[0]?.[0]?.text).toBe('Tiny')
        expect(rewrite.mock.calls[0]?.[0]?.whole_document).toBe(true)
    })

    it('clears the selection on a press outside the editor and keeps it on one inside', async () => {
        const wrapper = await mount_editor(MARKDOWN, rewrite_handler('x'))
        const editor = editor_of(wrapper)

        select(editor, 1, 16)
        await ai_button(wrapper).trigger('click')
        await wrapper.get('.rich-editor-ai-input').trigger('mousedown')

        expect(ai_meta(wrapper)).toBe('15 characters selected')

        document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
        await flushPromises()

        expect(editor.state.selection.empty).toBe(true)
        expect(ai_meta(wrapper)).toBe('Whole document')
    })

    it('shows the handler error message and stays open', async () => {
        const rewrite = vi.fn(() => Promise.reject(new Error('network error: offline')))
        const wrapper = await mount_editor(MARKDOWN, rewrite)

        await ai_button(wrapper).trigger('click')
        await quick_action(wrapper, 'Fix grammar').trigger('click')
        await flushPromises()

        expect(wrapper.get('.rich-editor-ai-error').text()).toBe('network error: offline')
        expect(wrapper.find('.rich-editor-ai').exists()).toBe(true)
    })

    it('reports an empty reply', async () => {
        const rewrite = vi.fn(() => Promise.resolve(''))
        const wrapper = await mount_editor(MARKDOWN, rewrite)

        await ai_button(wrapper).trigger('click')
        await quick_action(wrapper, 'Expand').trigger('click')
        await flushPromises()

        expect(wrapper.get('.rich-editor-ai-error').text()).toBe('The AI did not return any text.')
    })

    it('strips a code fence from the reply', async () => {
        const rewrite = rewrite_handler('```markdown\nFenced reply\n```')
        const wrapper = await mount_editor(MARKDOWN, rewrite)

        await ai_button(wrapper).trigger('click')
        await quick_action(wrapper, 'Rewrite').trigger('click')
        await flushPromises()

        expect(ipc.markdown_render_html).toHaveBeenCalledWith('Fenced reply')
        expect(model_emitted_last(wrapper)).toEqual(['Fenced reply'])
    })

    it('closes the panel and forgets the range from the close button', async () => {
        const wrapper = await mount_editor(MARKDOWN, rewrite_handler('x'))
        const editor = editor_of(wrapper)

        select(editor, 1, 6)
        await ai_button(wrapper).trigger('click')
        await wrapper.get('[aria-label="Close AI panel"]').trigger('click')

        expect(wrapper.find('.rich-editor-ai').exists()).toBe(false)

        collapse(editor, 1)
        await ai_button(wrapper).trigger('click')

        expect(ai_meta(wrapper)).toBe('Whole document')
    })

    it('hides the AI button without a handler', async () => {
        const wrapper = await mount_editor()

        expect(wrapper.find('.rich-tb-btn-ai').exists()).toBe(false)
    })
})
