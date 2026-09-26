import { ref } from 'vue'


export type DialogKind = 'info' | 'warning' | 'error'

export interface DialogConfirmOptions {
    title?: string
    kind?: DialogKind
    ok_label?: string
    cancel_label?: string
}

export interface DialogMessageOptions {
    title?: string
    kind?: DialogKind
    ok_label?: string
}

interface DialogFields {
    id: string
    title: string
    message: string
    kind: DialogKind
    ok_label: string
    cancel_label: string | null
    danger: boolean
}

interface DialogState extends DialogFields {
    resolve: (value: boolean) => void
}

const TITLE_DEFAULT = 'scribe'
const OK_LABEL_DEFAULT = 'OK'
const CANCEL_LABEL_DEFAULT = 'Cancel'
const current = ref<DialogState | null>(null)


function show(fields: DialogFields): Promise<boolean> {
    const previous = current.value

    if (previous) previous.resolve(false)

    return new Promise<boolean>((resolve) => {
        current.value = { ...fields, resolve }
    })
}

function close(value: boolean) {
    const state = current.value

    if (!state) return

    current.value = null

    state.resolve(value)
}

function confirm(message: string, options?: DialogConfirmOptions): Promise<boolean> {
    const kind = options?.kind ?? 'warning'

    return show({
        id: crypto.randomUUID(),
        title: options?.title ?? TITLE_DEFAULT,
        message,
        kind,
        ok_label: options?.ok_label ?? OK_LABEL_DEFAULT,
        cancel_label: options?.cancel_label ?? CANCEL_LABEL_DEFAULT,
        danger: kind === 'warning' || kind === 'error',
    })
}

async function message(text: string, options?: DialogMessageOptions): Promise<void> {
    await show({
        id: crypto.randomUUID(),
        title: options?.title ?? TITLE_DEFAULT,
        message: text,
        kind: options?.kind ?? 'info',
        ok_label: options?.ok_label ?? OK_LABEL_DEFAULT,
        cancel_label: null,
        danger: false,
    })
}

export function use_dialog() {
    return { current, confirm, message, close }
}
