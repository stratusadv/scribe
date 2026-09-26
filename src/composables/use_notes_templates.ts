import { ref } from 'vue'
import { ipc } from '../lib/ipc'
import { use_endpoints } from './use_endpoints'
import type { NotesTemplate } from '../types'


const MESSAGE_KEY_MISSING = 'Add your notes API key under Settings first.'
const templates = ref<NotesTemplate[]>([])
const error_message = ref<string | null>(null)


async function refresh() {
    error_message.value = null

    try {
        templates.value = await ipc.notes_templates_list()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function save(template: NotesTemplate) {
    error_message.value = null

    try {
        await ipc.notes_template_save(template)
        await refresh()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function remove(id: string) {
    error_message.value = null

    try {
        await ipc.notes_template_delete(id)
        await refresh()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function generate(description: string): Promise<string> {
    const { endpoints_by_purpose } = use_endpoints()
    const endpoint_id = endpoints_by_purpose('notes')[0]?.id

    if (!endpoint_id) throw new Error(MESSAGE_KEY_MISSING)

    return await ipc.notes_template_generate(endpoint_id, description)
}

export function use_notes_templates() {
    return { templates, error_message, refresh, save, remove, generate }
}
