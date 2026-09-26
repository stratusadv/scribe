const enabled = typeof performance !== 'undefined' && typeof performance.mark === 'function'

export function performance_mark(name: string) {
    if (!enabled) return

    performance.mark(name)
}

export function performance_measure(name: string, mark_start?: string) {
    if (!enabled) return

    try {
        const entry = performance.measure(name, mark_start)

        console.info(`[perf] ${name} ${entry.duration.toFixed(1)}ms`)
    } catch {
        return
    }
}
