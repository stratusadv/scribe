import { use_dialog } from '../composables/use_dialog'


type ErrorKind = 'network' | 'api' | 'config' | 'unknown'

interface FriendlyError {
    title: string
    message: string
    kind: ErrorKind
}

const PREFIX_NETWORK = 'network error:'
const PREFIX_API = 'api error:'
const PREFIX_CONFIG = 'config error:'
const MARKER_STOPPED_MIDWAY = 'decoding response body'


export function error_friendly(error: unknown): FriendlyError {
    const raw = error_text_extract(error)
    const lower = raw.toLowerCase()

    if (lower.startsWith(PREFIX_NETWORK)) {
        return error_friendly_network(raw, lower)
    }

    if (lower.startsWith(PREFIX_API)) {
        return {
            title: 'The server rejected the request',
            message: error_strip_prefix(raw, PREFIX_API),
            kind: 'api',
        }
    }

    if (lower.startsWith(PREFIX_CONFIG)) {
        return {
            title: 'Configuration issue',
            message: error_strip_prefix(raw, PREFIX_CONFIG),
            kind: 'config',
        }
    }

    return {
        title: 'Something went wrong',
        message: raw,
        kind: 'unknown',
    }
}

function error_friendly_network(raw: string, lower: string): FriendlyError {
    const detail = error_strip_prefix(raw, PREFIX_NETWORK)

    if (lower.includes(MARKER_STOPPED_MIDWAY)) {
        return {
            title: 'The server stopped responding',
            message: 'The server stopped part-way through its reply. It may be busy; wait a '
                + `minute and try again. (${detail})`,
            kind: 'network',
        }
    }

    return {
        title: 'Network problem',
        message: 'The server could not be reached. Check your internet connection and try again. '
            + `(${detail})`,
        kind: 'network',
    }
}

export async function error_dialog_show(error: unknown): Promise<void> {
    const friendly = error_friendly(error)
    const { message } = use_dialog()
    const kind = friendly.kind === 'network' || friendly.kind === 'unknown' ? 'error' : 'warning'

    await message(friendly.message, { title: friendly.title, kind })
}

export function error_text_extract(error: unknown): string {
    if (typeof error === 'string') return error
    if (error instanceof Error) return error.message

    switch (typeof error) {
        case 'object':
            return error === null ? 'null' : error_object_text(error)
        case 'undefined':
            return 'undefined'
        case 'function':
            return error.name
        case 'symbol':
            return error.description ?? 'symbol'
        default:
            return String(error)
    }
}

function error_object_text(error: object): string {
    const maybe = (error as { message?: unknown }).message

    if (typeof maybe === 'string') return maybe

    return JSON.stringify(error)
}

function error_strip_prefix(raw: string, prefix: string): string {
    if (raw.toLowerCase().startsWith(prefix.toLowerCase())) {
        return raw.slice(prefix.length).trim()
    }

    return raw
}
