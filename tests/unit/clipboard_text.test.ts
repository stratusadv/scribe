import { describe, expect, it } from 'vitest'
import { clipboard_plain_text } from '../../src/lib/clipboard_text'
import type { ClipboardNode } from '../../src/lib/clipboard_text'


type Attrs = Record<string, unknown>

function node(type: string, children: ClipboardNode[] = [], attrs: Attrs = {}): ClipboardNode {
    return {
        attrs,
        content: {
            forEach: (callback) => {
                children.forEach(callback)
            },
        },
        isText: false,
        text: undefined,
        textContent: children.map((child) => child.textContent).join(''),
        type: { name: type },
    }
}

function text(value: string): ClipboardNode {
    return {
        attrs: {},
        content: { forEach: () => undefined },
        isText: true,
        text: value,
        textContent: value,
        type: { name: 'text' },
    }
}

function hard_break(): ClipboardNode {
    return node('hardBreak')
}

function paragraph(...inline: (string | ClipboardNode)[]): ClipboardNode {
    return node('paragraph', inline.map((part) => (typeof part === 'string' ? text(part) : part)))
}

function heading(value: string, level: number): ClipboardNode {
    return node('heading', [text(value)], { level })
}

function code_block(value: string): ClipboardNode {
    return node('codeBlock', [text(value)])
}

function list_item(...blocks: ClipboardNode[]): ClipboardNode {
    return node('listItem', blocks)
}

function bullet_list(...items: ClipboardNode[]): ClipboardNode {
    return node('bulletList', items)
}

function ordered_list(items: ClipboardNode[], attrs: Attrs = {}): ClipboardNode {
    return node('orderedList', items, attrs)
}

function task_item(checked: boolean, ...blocks: ClipboardNode[]): ClipboardNode {
    return node('taskItem', blocks, { checked })
}

function fragment(...blocks: ClipboardNode[]): ClipboardNode['content'] {
    return {
        forEach: (callback) => {
            blocks.forEach(callback)
        },
    }
}

describe('clipboard_plain_text', () => {
    it('joins paragraphs with a blank line', () => {
        const result = clipboard_plain_text(fragment(paragraph('First'), paragraph('Second')))

        expect(result).toBe('First\n\nSecond')
    })

    it('returns an empty string for an empty fragment', () => {
        expect(clipboard_plain_text(fragment())).toBe('')
    })

    it('drops blocks that produce no lines', () => {
        const result = clipboard_plain_text(
            fragment(paragraph('One'), node('horizontalRule'), paragraph('Two')),
        )

        expect(result).toBe('One\n\nTwo')
    })

    it('turns a hard break into a newline within a paragraph', () => {
        const block = paragraph('line one', hard_break(), 'line two')
        const result = clipboard_plain_text(fragment(block))

        expect(result).toBe('line one\nline two')
    })

    it('keeps heading text without any markdown marker', () => {
        expect(clipboard_plain_text(fragment(heading('Summary', 2)))).toBe('Summary')
    })

    it('keeps code block lines verbatim', () => {
        const result = clipboard_plain_text(fragment(code_block('let x = 1\nlet y = 2')))

        expect(result).toBe('let x = 1\nlet y = 2')
    })

    it('uses the text content of an unknown inline-bearing node', () => {
        const mention = node('mention', [text('@sam')])

        expect(clipboard_plain_text(fragment(paragraph('Ask ', mention)))).toBe('Ask @sam')
    })

    it('skips an unknown node with no text', () => {
        expect(clipboard_plain_text(fragment(node('image')))).toBe('')
    })

    it('prefixes bullet items with a dash', () => {
        const result = clipboard_plain_text(fragment(bullet_list(
            list_item(paragraph('apples')),
            list_item(paragraph('pears')),
        )))

        expect(result).toBe('- apples\n- pears')
    })

    it('numbers ordered items from one by default', () => {
        const result = clipboard_plain_text(fragment(ordered_list([
            list_item(paragraph('first')),
            list_item(paragraph('second')),
        ])))

        expect(result).toBe('1. first\n2. second')
    })

    it('honours the start attribute of an ordered list', () => {
        const result = clipboard_plain_text(fragment(ordered_list(
            [list_item(paragraph('nine')), list_item(paragraph('ten'))],
            { start: 9 },
        )))

        expect(result).toBe('9. nine\n10. ten')
    })

    it('indents a nested list by two spaces per level', () => {
        const result = clipboard_plain_text(fragment(bullet_list(
            list_item(
                paragraph('fruit'),
                ordered_list([
                    list_item(
                        paragraph('apples'),
                        bullet_list(list_item(paragraph('granny smith'))),
                    ),
                    list_item(paragraph('pears')),
                ]),
            ),
            list_item(paragraph('bread')),
        )))

        expect(result).toBe([
            '- fruit',
            '  1. apples',
            '    - granny smith',
            '  2. pears',
            '- bread',
        ].join('\n'))
    })

    it('aligns continuation lines under the item text', () => {
        const result = clipboard_plain_text(fragment(bullet_list(
            list_item(paragraph('first', hard_break(), 'wrapped'), paragraph('second paragraph')),
        )))

        expect(result).toBe('- first\n  wrapped\n  second paragraph')
    })

    it('marks task items with their checked state', () => {
        const result = clipboard_plain_text(fragment(node('taskList', [
            task_item(true, paragraph('done')),
            task_item(false, paragraph('open')),
        ])))

        expect(result).toBe('[x] done\n[ ] open')
    })

    it('flattens a blockquote into its paragraphs', () => {
        const result = clipboard_plain_text(fragment(
            node('blockquote', [paragraph('quoted'), paragraph('again')]),
        ))

        expect(result).toBe('quoted\nagain')
    })

    it('renders table rows as tab separated cells', () => {
        const cell = (...blocks: ClipboardNode[]) => node('tableCell', blocks)
        const row = (...cells: ClipboardNode[]) => node('tableRow', cells)

        const table = node('table', [
            row(cell(paragraph('Who')), cell(paragraph('What'))),
            row(cell(paragraph('Sam')), cell(paragraph('copy', hard_break(), 'draft'))),
        ])

        expect(clipboard_plain_text(fragment(table))).toBe('Who\tWhat\nSam\tcopy draft')
    })

    it('stops visiting nodes once the node budget is spent', () => {
        const items: ClipboardNode[] = []

        for (let index = 0; index < 60_000; index += 1) {
            items.push(list_item(paragraph('x')))
        }

        const lines = clipboard_plain_text(fragment(bullet_list(...items))).split('\n')

        expect(lines.length).toBe(49_999)
        expect(lines[0]).toBe('- x')
        expect(lines[49_998]).toBe('- x')
    })
})
