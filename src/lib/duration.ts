import { assert } from './assert'


const SECONDS_PER_MINUTE = 60
const SECONDS_PER_HOUR = 3600
const MILLISECONDS_PER_SECOND = 1000

function seconds_valid(seconds: number): boolean {
    return Number.isFinite(seconds) && seconds >= 0
}

export function seconds_to_clock(seconds: number): string {
    if (!seconds_valid(seconds)) return '0:00'

    const minutes = Math.floor(seconds / SECONDS_PER_MINUTE)
    const seconds_rest = Math.floor(seconds % SECONDS_PER_MINUTE)

    return `${minutes}:${String(seconds_rest).padStart(2, '0')}`
}

export function seconds_to_clock_hours(seconds: number): string {
    if (!seconds_valid(seconds)) return '0:00'

    const hours = Math.floor(seconds / SECONDS_PER_HOUR)
    const minutes = Math.floor((seconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE)
    const seconds_rest = Math.floor(seconds % SECONDS_PER_MINUTE)
    const minutes_text = hours > 0 ? String(minutes).padStart(2, '0') : String(minutes)
    const minutes_seconds = `${minutes_text}:${String(seconds_rest).padStart(2, '0')}`

    return hours > 0 ? `${hours}:${minutes_seconds}` : minutes_seconds
}

export function unix_now(): number {
    const now = Math.floor(Date.now() / MILLISECONDS_PER_SECOND)

    assert(now >= 0, 'the clock reads a time before the epoch')

    return now
}
