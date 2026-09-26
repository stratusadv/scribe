export interface ClipboardNode {
    attrs: Readonly<Record<string, unknown>>
    content: { forEach(callback: (node: ClipboardNode) => void): void }
    isText: boolean
    text: string | undefined
    textContent: string
    type: { name: string }
}

interface Entry {
    indent: number
    node: ClipboardNode
    prefix: string
}

const LIST_INDENT = 2
const NODE_COUNT_MAX = 100_000

export function clipboard_plain_text(fragment: ClipboardNode['content']): string {
    const blocks: string[] = []

    fragment.forEach((block) => {
        const lines = block_lines(block)

        if (lines.length > 0) blocks.push(lines.join('\n'))
    })

    return blocks.join('\n\n')
}

function block_lines(block: ClipboardNode): string[] {
    const lines: string[] = []
    const stack: Entry[] = [{ node: block, indent: 0, prefix: '' }]
    let visited = 0

    while (visited < NODE_COUNT_MAX) {
        const entry = stack.pop()
        if (!entry) break

        visited += 1

        const { node, indent, prefix } = entry

        switch (node.type.name) {
            case 'paragraph':
            case 'heading':
                lines.push(...text_lines(inline_text(node), indent, prefix))

                break
            case 'codeBlock':
                lines.push(...text_lines(node.textContent, indent, prefix))

                break
            case 'bulletList':
            case 'orderedList':
            case 'taskList':
                stack.push(...list_entries(node, indent).reverse())

                break
            case 'listItem':
            case 'taskItem':
                stack.push(...item_entries(node, indent, prefix).reverse())

                break
            case 'blockquote':
                stack.push(...children_entries(node, indent, prefix).reverse())

                break
            case 'table':
                lines.push(...table_lines(node, indent))

                break
            case 'horizontalRule':
            case 'hardBreak':
                break
            default:
                if (node.textContent.length > 0) {
                    lines.push(...text_lines(node.textContent, indent, prefix))
                }
        }
    }

    return lines
}

function children_entries(node: ClipboardNode, indent: number, prefix: string): Entry[] {
    const entries: Entry[] = []

    node.content.forEach((child) => {
        entries.push({ node: child, indent, prefix: entries.length === 0 ? prefix : '' })
    })

    return entries
}

function inline_text(node: ClipboardNode): string {
    let text = ''

    node.content.forEach((child) => {
        if (child.isText) {
            text += child.text ?? ''
        } else if (child.type.name === 'hardBreak') {
            text += '\n'
        } else {
            text += child.textContent
        }
    })

    return text
}

function item_entries(item: ClipboardNode, indent: number, prefix: string): Entry[] {
    const marker = item.type.name === 'taskItem'
        ? (item.attrs['checked'] === true ? '[x] ' : '[ ] ')
        : prefix

    const entries: Entry[] = []

    item.content.forEach((child) => {
        const is_first = entries.length === 0

        entries.push({
            node: child,
            indent: is_first ? indent : indent + LIST_INDENT,
            prefix: is_first ? marker : '',
        })
    })

    return entries
}

function list_entries(list: ClipboardNode, indent: number): Entry[] {
    const start = typeof list.attrs['start'] === 'number' ? list.attrs['start'] : 1
    const entries: Entry[] = []

    list.content.forEach((item) => {
        const ordinal = start + entries.length
        const marker = list.type.name === 'orderedList' ? `${String(ordinal)}. ` : '- '

        entries.push({ node: item, indent, prefix: marker })
    })

    return entries
}

function table_lines(table: ClipboardNode, indent: number): string[] {
    const pad = ' '.repeat(indent)
    const lines: string[] = []

    table.content.forEach((row) => {
        const cells: string[] = []

        row.content.forEach((cell) => {
            const paragraphs: string[] = []

            cell.content.forEach((paragraph) => {
                paragraphs.push(inline_text(paragraph).replace(/\n/g, ' '))
            })

            cells.push(paragraphs.join(' '))
        })

        lines.push(pad + cells.join('\t'))
    })

    return lines
}

function text_lines(text: string, indent: number, prefix: string): string[] {
    const pad = ' '.repeat(indent)
    const continuation = pad + ' '.repeat(prefix.length)

    return text.split('\n').map((line, index) => (index === 0 ? pad + prefix : continuation) + line)
}
