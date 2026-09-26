<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import { generateJSON } from '@tiptap/core'
import type { Content, Editor, JSONContent } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import Link from '@tiptap/extension-link'
import TaskList from '@tiptap/extension-task-list'
import TaskItem from '@tiptap/extension-task-item'
import Placeholder from '@tiptap/extension-placeholder'
import { Table, TableRow, TableCell, TableHeader } from '@tiptap/extension-table'
import { Markdown } from 'tiptap-markdown'
import { use_dialog } from '../composables/use_dialog'
import { clipboard_plain_text } from '../lib/clipboard_text'
import { error_text_extract } from '../lib/errors'
import { ipc } from '../lib/ipc'
import { performance_mark, performance_measure } from '../lib/performance'
import type { AIGenerateHandler, AIRewriteHandler, AIRewriteArgs } from '../types'


type HeadingLevel = 1 | 2 | 3

interface AIQuickAction {
    id: string
    label: string
    instruction: string
}

interface MarkdownStorage {
    getMarkdown(): string
}

type MarkdownStorageTable = Record<string, MarkdownStorage | undefined>
const HEADING_LEVELS: HeadingLevel[] = [1, 2, 3]
const CODE_FENCE = '```'
const TOOLBAR_COLLAPSE_WIDTH_PX = 640
const MESSAGE_INSTRUCTION_MISSING = 'Enter an instruction.'
const MESSAGE_EMPTY_DOCUMENT = 'Nothing to rewrite yet.'
const MESSAGE_EMPTY_REPLY = 'The AI did not return any text.'
const MESSAGE_EMPTY_LAYOUT = 'The AI did not return anything.'
const MESSAGE_DESCRIBE_FIRST = 'Describe the document first.'
const HTML_BLOCK_START = /^<(?:p|h[1-6]|ul|ol|li|blockquote|pre|table|hr|div|span|strong|em|code|br)\b/i

const AI_QUICK_ACTIONS: AIQuickAction[] = [
    {
        id: 'rewrite',
        label: 'Rewrite',
        instruction: 'Rewrite this more clearly and concisely. Keep the same meaning.',
    },
    {
        id: 'shorten',
        label: 'Shorten',
        instruction: 'Make this shorter without losing the key points.',
    },
    {
        id: 'expand',
        label: 'Expand',
        instruction: 'Expand this with more detail and context.',
    },
    {
        id: 'grammar',
        label: 'Fix grammar',
        instruction: 'Fix grammar, spelling, and punctuation. Do not change the meaning.',
    },
    {
        id: 'formal',
        label: 'More formal',
        instruction: 'Rewrite in a more formal, professional tone.',
    },
    {
        id: 'bullets',
        label: 'To bullets',
        instruction: 'Rewrite as a markdown bullet list. One bullet per distinct point.',
    },
]

const props = defineProps<{
    modelValue: string
    placeholder?: string
    ai_rewrite?: AIRewriteHandler
    ai_generate?: AIGenerateHandler
}>()

const emit = defineEmits<{
    'update:modelValue': [value: string]
}>()

const { confirm: dialog_confirm } = use_dialog()
const fullscreen = ref(false)
const source_mode = ref(false)
const source_buffer = ref('')
const ai_panel_open = ref(false)
const ai_busy = ref(false)
const ai_instruction = ref('')
const ai_generate_description = ref('')
const ai_selected_preview = ref('')
const ai_error = ref('')
const has_selection = ref(false)
const is_in_table = ref(false)
const table_bar_dismissed = ref(false)
const toolbar_collapsed = ref(false)
const more_open = ref(false)
const more_panel_id = useId()
const root_ref = ref<HTMLElement | null>(null)
let resize_observer: ResizeObserver | null = null
const heading_level_active = ref<string>('')
let markdown_emitted = props.modelValue

function markdown_of(editor_current: Editor): string {
    const storage = editor_current.storage as unknown as MarkdownStorageTable

    return storage['markdown']?.getMarkdown() ?? ''
}

function heading_level_of(editor_current: Editor): string {
    const level = HEADING_LEVELS.find((candidate) => editor_current.isActive('heading', { level: candidate }))

    return level === undefined ? '' : String(level)
}

function refresh_state(editor_current: Editor) {
    const { from, to } = editor_current.state.selection

    has_selection.value = to > from
    is_in_table.value = editor_current.isActive('table')
    more_open.value = false

    if (!is_in_table.value) table_bar_dismissed.value = false

    heading_level_active.value = heading_level_of(editor_current)

    if (!ai_panel_open.value) return

    ai_selected_preview.value = to === from
        ? ''
        : editor_current.state.doc.textBetween(from, to, '\n')
}

const editor = useEditor({
    content: props.modelValue,
    extensions: [
        StarterKit.configure({ link: false }),
        Link.configure({ openOnClick: false, autolink: true }),
        TaskList,
        TaskItem.configure({ nested: true }),
        Placeholder.configure({ placeholder: props.placeholder ?? 'Start writing…' }),
        Table.configure({ resizable: false }),
        TableRow,
        TableHeader,
        TableCell,
        Markdown.configure({ html: true, transformPastedText: true, breaks: true }),
    ],
    editorProps: {
        clipboardTextSerializer: (slice) => clipboard_plain_text(slice.content),
    },
    onUpdate({ editor: editor_current }) {
        performance_mark('editor_get_markdown_start')

        const markdown = markdown_of(editor_current)

        performance_measure('editor_get_markdown', 'editor_get_markdown_start')
        markdown_emitted = markdown

        if (markdown !== props.modelValue) emit('update:modelValue', markdown)

        refresh_state(editor_current)
    },
    onSelectionUpdate({ editor: editor_current }) {
        refresh_state(editor_current)
    },
})

const stats = computed(() => {
    const text = props.modelValue
    const trimmed = text.trim()
    const words = trimmed.length === 0 ? 0 : trimmed.split(/\s+/).length

    return { chars: text.length, words }
})

const ai_button_title = computed(() => {
    if (props.ai_generate) return 'Write the layout with AI'
    if (has_selection.value) return 'Rewrite the selected text with AI'

    return 'Rewrite the whole document with AI'
})

const ai_selected_count_text = computed(() => {
    const count = ai_selected_preview.value.length

    if (count === 0) return 'Whole document'

    return `${count} character${count === 1 ? '' : 's'} selected`
})

watch(
    () => props.modelValue,
    (next) => {
        if (source_mode.value) {
            source_buffer.value = next

            return
        }

        if (next === markdown_emitted) return

        content_apply()
    },
)

onMounted(() => {
    const root = root_ref.value

    if (!root) return

    resize_observer = new ResizeObserver((entries) => {
        const entry = entries[0]

        if (!entry) return

        toolbar_collapsed.value = entry.contentRect.width < TOOLBAR_COLLAPSE_WIDTH_PX

        if (!toolbar_collapsed.value) more_open.value = false
    })

    resize_observer.observe(root)
})

onBeforeUnmount(() => {
    resize_observer?.disconnect()
    editor.value?.destroy()
})

function content_apply() {
    const editor_current = editor.value

    if (!editor_current) return
    if (source_mode.value) return

    const next = props.modelValue

    if (next === markdown_emitted) return

    performance_mark('editor_set_content_start')
    editor_current.commands.setContent(next, { emitUpdate: false })
    performance_measure('editor_set_content', 'editor_set_content_start')
    markdown_emitted = next
}

function command_chain() {
    return editor.value?.chain().focus()
}

function is_active(name: string, attributes?: Record<string, unknown>): boolean {
    const editor_current = editor.value

    if (!editor_current) return false

    return attributes ? editor_current.isActive(name, attributes) : editor_current.isActive(name)
}

function toolbar_class(name: string): string[] {
    return ['rich-tb-btn', is_active(name) ? 'rich-tb-btn-active' : '']
}

function fullscreen_toggle() {
    fullscreen.value = !fullscreen.value
}

function source_enter() {
    source_buffer.value = props.modelValue
    source_mode.value = true
}

function source_exit() {
    const next = source_buffer.value
    const editor_current = editor.value

    source_mode.value = false

    emit('update:modelValue', next)

    if (editor_current) editor_current.commands.setContent(next, { emitUpdate: false })

    markdown_emitted = next
}

function source_input(event: Event) {
    source_buffer.value = (event.target as HTMLTextAreaElement).value

    emit('update:modelValue', source_buffer.value)
}

function heading_level_is(value: number): value is HeadingLevel {
    return HEADING_LEVELS.some((level) => level === value)
}

function heading_change(event: Event) {
    const value = (event.target as HTMLSelectElement).value
    const chain = command_chain()

    if (!chain) return

    if (value === '') {
        chain.setParagraph().run()

        return
    }

    const level = Number.parseInt(value, 10)

    if (heading_level_is(level)) chain.setHeading({ level }).run()
}

function link_apply() {
    const editor_current = editor.value

    if (!editor_current) return

    const link_attributes = editor_current.getAttributes('link') as Record<string, unknown>
    const href_previous = link_attributes['href']
    const previous = typeof href_previous === 'string' ? href_previous : 'https://'
    const url = window.prompt('Link URL', previous)

    if (url === null) return

    if (url.trim().length === 0) {
        editor_current.chain().focus().extendMarkRange('link').unsetLink().run()

        return
    }

    editor_current.chain().focus().extendMarkRange('link').setLink({ href: url }).run()
}

function table_insert() {
    if (is_in_table.value) {
        table_bar_dismissed.value = false

        return
    }

    command_chain()?.insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()
}

function ai_open() {
    if (props.ai_generate) {
        ai_error.value = ''
        ai_panel_open.value = true

        return
    }

    const editor_current = editor.value

    if (!props.ai_rewrite) return
    if (!editor_current) return

    const { from, to } = editor_current.state.selection

    ai_selected_preview.value = from === to
        ? ''
        : editor_current.state.doc.textBetween(from, to, '\n')

    ai_instruction.value = ''
    ai_error.value = ''
    ai_panel_open.value = true
}

function ai_close() {
    if (ai_busy.value) return

    ai_panel_open.value = false
    ai_instruction.value = ''
    ai_error.value = ''
    ai_selected_preview.value = ''
}

function ai_toggle() {
    if (ai_panel_open.value) {
        ai_close()

        return
    }

    ai_open()
}

function looks_like_html(text: string): boolean {
    return HTML_BLOCK_START.test(text.trimStart())
}

function sanitize_llm_output(text: string): string {
    const cleaned = text.trim()

    if (!cleaned.startsWith(CODE_FENCE) || !cleaned.endsWith(CODE_FENCE)) return cleaned

    return cleaned
        .replace(/^```[a-zA-Z0-9_-]*\n?/, '')
        .replace(/\n?```$/, '')
        .trim()
}

async function ai_rewrite_request(
    rewrite: AIRewriteHandler,
    text: string,
    instruction: string,
): Promise<string> {
    let accumulated = ''

    const args: AIRewriteArgs = {
        text,
        instruction,
        on_chunk: (chunk) => {
            accumulated += chunk
        },
    }

    const reply = await rewrite(args)

    return sanitize_llm_output(reply || accumulated)
}

async function ai_content_build(raw: string, editor_current: Editor): Promise<Content> {
    let html = raw

    if (!looks_like_html(raw)) {
        try {
            html = await ipc.markdown_render_html(raw)
        } catch (error) {
            console.warn('[editor] markdown render failed, inserting raw text', error)
        }
    }

    if (html.trim().length === 0) html = raw

    try {
        return generateJSON(html, editor_current.extensionManager.extensions) as JSONContent
    } catch (error) {
        console.warn('[editor] reply could not be parsed as a document', error)

        return html
    }
}

async function ai_apply(instruction: string) {
    const editor_current = editor.value
    const rewrite = props.ai_rewrite

    if (!editor_current || !rewrite || ai_busy.value) return

    const instruction_trimmed = instruction.trim()

    if (instruction_trimmed.length === 0) {
        ai_error.value = MESSAGE_INSTRUCTION_MISSING

        return
    }

    const { from, to } = editor_current.state.selection
    const whole_document = from === to

    const text_original = whole_document
        ? markdown_of(editor_current)
        : editor_current.state.doc.textBetween(from, to, '\n')

    if (text_original.trim().length === 0) {
        ai_error.value = MESSAGE_EMPTY_DOCUMENT

        return
    }

    ai_busy.value = true
    ai_error.value = ''

    try {
        const output_final = await ai_rewrite_request(rewrite, text_original, instruction_trimmed)
        const editor_after = editor.value

        if (!editor_after || output_final.length === 0) {
            ai_error.value = MESSAGE_EMPTY_REPLY

            return
        }

        const content = await ai_content_build(output_final, editor_after)

        if (whole_document) {
            editor_after.commands.setContent(content, { emitUpdate: true })
        } else {
            editor_after
                .chain()
                .focus()
                .setTextSelection({ from, to })
                .deleteSelection()
                .insertContent(content)
                .run()

            ai_selected_preview.value = output_final
        }
    } catch (error) {
        ai_error.value = error_text_extract(error)
    } finally {
        ai_busy.value = false
    }
}

async function ai_generate_confirm(): Promise<boolean> {
    if (props.modelValue.trim().length === 0) return true

    return await dialog_confirm(
        'Replace what is in the editor with a layout written by the AI?',
        { title: 'Replace layout', kind: 'warning' },
    )
}

async function ai_generate_apply() {
    const editor_current = editor.value
    const generate = props.ai_generate

    if (!editor_current || !generate || ai_busy.value) return

    const description = ai_generate_description.value.trim()

    if (description.length === 0) {
        ai_error.value = MESSAGE_DESCRIBE_FIRST

        return
    }

    if (!(await ai_generate_confirm())) return

    ai_busy.value = true
    ai_error.value = ''

    try {
        const generated = sanitize_llm_output(await generate(description))

        if (generated.length === 0) {
            ai_error.value = MESSAGE_EMPTY_LAYOUT

            return
        }

        editor_current.commands.setContent(generated)
        ai_panel_open.value = false
        ai_generate_description.value = ''
    } catch (error) {
        ai_error.value = error_text_extract(error)
    } finally {
        ai_busy.value = false
    }
}

function ai_quick_apply(action: AIQuickAction) {
    void ai_apply(action.instruction)
}

function ai_custom_apply() {
    void ai_apply(ai_instruction.value)
}
</script>

<template>
    <div ref="root_ref" class="rich-editor" :data-fullscreen="fullscreen ? 'true' : 'false'">
        <div class="rich-editor-toolbar">
            <div
                :id="more_panel_id"
                class="rich-tb-more-panel"
                :data-open="more_open ? 'true' : 'false'"
            />
            <select
                :value="heading_level_active"
                class="rich-tb-select"
                :disabled="source_mode"
                @change="heading_change"
            >
                <option value="">Normal text</option>
                <option value="1">Heading 1</option>
                <option value="2">Heading 2</option>
                <option value="3">Heading 3</option>
            </select>
            <span class="rich-tb-sep" />
            <button
                type="button"
                :class="toolbar_class('bold')"
                title="Bold (Ctrl+B)"
                :disabled="source_mode"
                @mousedown.prevent
                @click="command_chain()?.toggleBold().run()"
            >
                <strong>B</strong>
            </button>
            <button
                type="button"
                :class="toolbar_class('italic')"
                title="Italic (Ctrl+I)"
                :disabled="source_mode"
                @mousedown.prevent
                @click="command_chain()?.toggleItalic().run()"
            >
                <em>I</em>
            </button>
            <Teleport defer :to="`#${more_panel_id}`" :disabled="!toolbar_collapsed">
                <button
                    type="button"
                    :class="toolbar_class('strike')"
                    title="Strikethrough"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="command_chain()?.toggleStrike().run()"
                >
                    <span style="text-decoration: line-through">S</span>
                </button>
                <button
                    type="button"
                    :class="toolbar_class('code')"
                    title="Inline code"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="command_chain()?.toggleCode().run()"
                >
                    <code>&lt;&gt;</code>
                </button>
            </Teleport>
            <span class="rich-tb-sep" />
            <Teleport defer :to="`#${more_panel_id}`" :disabled="!toolbar_collapsed">
                <button
                    type="button"
                    :class="toolbar_class('bulletList')"
                    title="Bullet list"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="command_chain()?.toggleBulletList().run()"
                >
                    •&nbsp;List
                </button>
                <button
                    type="button"
                    :class="toolbar_class('orderedList')"
                    title="Numbered list"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="command_chain()?.toggleOrderedList().run()"
                >
                    1.&nbsp;List
                </button>
                <button
                    type="button"
                    :class="toolbar_class('taskList')"
                    title="Checklist"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="command_chain()?.toggleTaskList().run()"
                >
                    ☑&nbsp;List
                </button>
                <span class="rich-tb-sep" />
                <button
                    type="button"
                    :class="toolbar_class('blockquote')"
                    title="Quote"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="command_chain()?.toggleBlockquote().run()"
                >
                    &ldquo;&nbsp;&rdquo;
                </button>
                <button
                    type="button"
                    :class="toolbar_class('codeBlock')"
                    title="Code block"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="command_chain()?.toggleCodeBlock().run()"
                >
                    { }
                </button>
                <button
                    type="button"
                    class="rich-tb-btn"
                    title="Horizontal rule"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="command_chain()?.setHorizontalRule().run()"
                >
                    ―
                </button>
                <span class="rich-tb-sep" />
                <button
                    type="button"
                    :class="toolbar_class('link')"
                    title="Link"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="link_apply"
                >
                    Link
                </button>
                <button
                    type="button"
                    class="rich-tb-btn"
                    :title="is_in_table ? 'Show table tools' : 'Insert table'"
                    :disabled="source_mode"
                    @mousedown.prevent
                    @click="table_insert"
                >
                    Table
                </button>
            </Teleport>
            <button
                v-if="toolbar_collapsed"
                type="button"
                :class="more_open ? 'rich-tb-btn rich-tb-btn-active' : 'rich-tb-btn'"
                :aria-expanded="more_open"
                title="More tools"
                @mousedown.prevent
                @click="more_open = !more_open"
            >
                More ▾
            </button>
            <button
                v-if="ai_rewrite || ai_generate"
                type="button"
                class="rich-tb-btn rich-tb-btn-ai"
                :aria-expanded="ai_panel_open"
                :title="ai_button_title"
                :disabled="source_mode"
                @mousedown.prevent
                @click="ai_toggle"
            >
                ✨ AI
            </button>
            <span class="flex-1" />
            <Teleport defer :to="`#${more_panel_id}`" :disabled="!toolbar_collapsed">
                <button
                    v-if="!source_mode"
                    type="button"
                    class="rich-tb-btn"
                    title="Edit raw markdown"
                    @click="source_enter"
                >
                    Markdown
                </button>
                <button
                    v-else
                    type="button"
                    class="rich-tb-btn rich-tb-btn-active"
                    title="Back to rich editor"
                    @click="source_exit"
                >
                    Rich editor
                </button>
            </Teleport>
            <button
                type="button"
                class="rich-tb-btn"
                :title="fullscreen ? 'Exit fullscreen' : 'Fullscreen'"
                @mousedown.prevent
                @click="fullscreen_toggle"
            >
                {{ fullscreen ? 'Exit' : 'Fullscreen' }}
            </button>
        </div>

        <div
            v-if="!source_mode && is_in_table && !table_bar_dismissed"
            class="rich-editor-context-bar"
        >
            <span class="rich-editor-context-label">Table</span>
            <button
                type="button"
                class="rich-tb-btn"
                title="Add row above"
                @mousedown.prevent
                @click="command_chain()?.addRowBefore().run()"
            >
                + Row above
            </button>
            <button
                type="button"
                class="rich-tb-btn"
                title="Add row below"
                @mousedown.prevent
                @click="command_chain()?.addRowAfter().run()"
            >
                + Row below
            </button>
            <button
                type="button"
                class="rich-tb-btn"
                title="Add column left"
                @mousedown.prevent
                @click="command_chain()?.addColumnBefore().run()"
            >
                + Column left
            </button>
            <button
                type="button"
                class="rich-tb-btn"
                title="Add column right"
                @mousedown.prevent
                @click="command_chain()?.addColumnAfter().run()"
            >
                + Column right
            </button>
            <span class="rich-tb-sep" />
            <button
                type="button"
                class="rich-tb-btn"
                title="Toggle header row"
                @mousedown.prevent
                @click="command_chain()?.toggleHeaderRow().run()"
            >
                Toggle header
            </button>
            <button
                type="button"
                class="rich-tb-btn"
                title="Merge cells"
                @mousedown.prevent
                @click="command_chain()?.mergeCells().run()"
            >
                Merge
            </button>
            <button
                type="button"
                class="rich-tb-btn"
                title="Split cell"
                @mousedown.prevent
                @click="command_chain()?.splitCell().run()"
            >
                Split
            </button>
            <span class="flex-1" />
            <button
                type="button"
                class="rich-tb-btn"
                title="Delete row"
                @mousedown.prevent
                @click="command_chain()?.deleteRow().run()"
            >
                Delete row
            </button>
            <button
                type="button"
                class="rich-tb-btn"
                title="Delete column"
                @mousedown.prevent
                @click="command_chain()?.deleteColumn().run()"
            >
                Delete column
            </button>
            <button
                type="button"
                class="rich-tb-btn rich-tb-btn-danger"
                title="Delete entire table"
                @mousedown.prevent
                @click="command_chain()?.deleteTable().run()"
            >
                Delete table
            </button>
            <span class="rich-tb-sep" />
            <button
                type="button"
                class="rich-tb-btn rich-tb-btn-icon"
                title="Hide the table tools until you leave the table"
                aria-label="Close table tools"
                @mousedown.prevent
                @click="table_bar_dismissed = true"
            >
                <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.75"
                    stroke-linecap="round"
                    aria-hidden="true"
                >
                    <path d="M6 6l12 12" />
                    <path d="M18 6L6 18" />
                </svg>
            </button>
        </div>

        <div v-if="!source_mode && ai_panel_open && ai_generate" class="rich-editor-ai">
            <div v-if="ai_busy" class="rich-editor-ai-busy">
                <span class="rich-editor-ai-spinner" />
                <span class="rich-editor-ai-busy-label">Generating…</span>
            </div>
            <div class="rich-editor-ai-custom">
                <input
                    v-model="ai_generate_description"
                    type="text"
                    class="input rich-editor-ai-input"
                    maxlength="500"
                    placeholder="Describe the document, e.g. weekly team meeting: decisions, who does what by when, problems to watch"
                    :disabled="ai_busy"
                    @keydown.enter.prevent="ai_generate_apply"
                />
                <button
                    type="button"
                    class="btn-primary"
                    :disabled="ai_busy || ai_generate_description.trim().length === 0"
                    @mousedown.prevent
                    @click="ai_generate_apply"
                >
                    {{ ai_busy ? 'Generating…' : 'Generate' }}
                </button>
            </div>
            <p v-if="ai_error" class="rich-editor-ai-error">{{ ai_error }}</p>
        </div>

        <div v-else-if="!source_mode && ai_panel_open" class="rich-editor-ai">
            <div class="rich-editor-ai-header">
                <span class="rich-editor-ai-title">AI rewrite</span>
                <span class="rich-editor-ai-meta opacity-60">·</span>
                <span class="rich-editor-ai-meta">{{ ai_selected_count_text }}</span>
                <span class="flex-1" />
                <button
                    type="button"
                    class="rich-tb-btn rich-tb-btn-icon"
                    title="Close"
                    aria-label="Close AI panel"
                    :disabled="ai_busy"
                    @mousedown.prevent
                    @click="ai_close"
                >
                    <svg
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.75"
                        stroke-linecap="round"
                        aria-hidden="true"
                    >
                        <path d="M6 6l12 12" />
                        <path d="M18 6L6 18" />
                    </svg>
                </button>
            </div>
            <div class="rich-editor-ai-actions">
                <button
                    v-for="action in AI_QUICK_ACTIONS"
                    :key="action.id"
                    type="button"
                    class="rich-tb-btn"
                    :disabled="ai_busy"
                    @mousedown.prevent
                    @click="ai_quick_apply(action)"
                >
                    {{ action.label }}
                </button>
            </div>
            <div class="rich-editor-ai-custom">
                <input
                    v-model="ai_instruction"
                    type="text"
                    class="input rich-editor-ai-input"
                    placeholder="Custom instruction (e.g. 'rewrite as bullet points')"
                    :disabled="ai_busy"
                    @keydown.enter.prevent="ai_custom_apply"
                />
                <button
                    type="button"
                    class="btn-primary"
                    :disabled="ai_busy || ai_instruction.trim().length === 0"
                    @mousedown.prevent
                    @click="ai_custom_apply"
                >
                    {{ ai_busy ? 'Working…' : 'Apply' }}
                </button>
            </div>
            <p v-if="ai_error" class="rich-editor-ai-error">{{ ai_error }}</p>
        </div>

        <div v-if="!source_mode" class="rich-editor-body">
            <EditorContent v-if="editor" :editor="editor" class="rich-editor-content prose-render" />
        </div>
        <textarea
            v-else
            class="rich-editor-source"
            spellcheck="false"
            :value="source_buffer"
            @input="source_input"
        />

        <div class="rich-editor-stats">
            <span>{{ stats.words }} word{{ stats.words === 1 ? '' : 's' }}</span>
            <span class="opacity-60">·</span>
            <span>{{ stats.chars }} character{{ stats.chars === 1 ? '' : 's' }}</span>
            <slot name="status" />
        </div>
    </div>
</template>
