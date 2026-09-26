import { onActivated, onDeactivated, onMounted, onUnmounted } from 'vue'


type Handler = (event: KeyboardEvent) => void

interface ShortcutRegistration {
    shortcuts: Record<string, Handler>
    registered: boolean
}

const INPUT_TYPES_NON_TEXT = Object.freeze([
    'button',
    'checkbox',
    'radio',
    'submit',
    'reset',
    'range',
    'color',
    'file',
])

const handlers_by_combo = new Map<string, Handler[]>()


function event_has_control(event: KeyboardEvent): boolean {
    if (event.ctrlKey) return true

    return event.metaKey
}

function combo_key_from(event: KeyboardEvent): string {
    const parts: string[] = []

    if (event_has_control(event)) parts.push('ctrl')
    if (event.shiftKey) parts.push('shift')
    if (event.altKey) parts.push('alt')

    const key_lower = event.key.toLowerCase()
    const key_name = key_lower === ' ' ? 'space' : key_lower

    parts.push(key_name)

    return parts.join('+')
}

function event_target_is_text_input(event: KeyboardEvent): boolean {
    const target = event.target as HTMLElement | null

    if (!target) return false
    if (target.isContentEditable) return true

    const tag = target.tagName

    if (tag === 'TEXTAREA') return true

    if (tag === 'INPUT') {
        const input_type = (target as HTMLInputElement).type.toLowerCase()

        return !INPUT_TYPES_NON_TEXT.includes(input_type)
    }

    return false
}

function listener_global_keydown(event: KeyboardEvent) {
    const combo = combo_key_from(event)
    const handlers = handlers_by_combo.get(combo)

    if (!handlers) return

    if (!event_has_control(event)) {
        if (event_target_is_text_input(event)) return
    }

    const topmost = handlers[handlers.length - 1]

    if (!topmost) return

    event.preventDefault()
    topmost(event)
}

window.addEventListener('keydown', listener_global_keydown)

function registration_register(registration: ShortcutRegistration) {
    if (registration.registered) return

    registration.registered = true

    for (const [combo, handler] of Object.entries(registration.shortcuts)) {
        const existing = handlers_by_combo.get(combo) ?? []

        existing.push(handler)
        handlers_by_combo.set(combo, existing)
    }
}

function registration_unregister(registration: ShortcutRegistration) {
    if (!registration.registered) return

    registration.registered = false

    for (const [combo, handler] of Object.entries(registration.shortcuts)) {
        const existing = handlers_by_combo.get(combo)

        if (!existing) continue

        const index = existing.indexOf(handler)

        if (index >= 0) existing.splice(index, 1)
        if (existing.length === 0) void handlers_by_combo.delete(combo)
    }
}

export function use_shortcuts(shortcuts: Record<string, Handler>) {
    const registration: ShortcutRegistration = { shortcuts, registered: false }

    onMounted(() => {
        registration_register(registration)
    })

    onActivated(() => {
        registration_register(registration)
    })

    onDeactivated(() => {
        registration_unregister(registration)
    })

    onUnmounted(() => {
        registration_unregister(registration)
    })
}
