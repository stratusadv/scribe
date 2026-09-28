import { describe, expect, it } from 'vitest'
import { seconds_to_clock, seconds_to_clock_hours } from '../../src/lib/duration'


describe('seconds_to_clock', () => {
    it('formats minutes and zero-padded seconds', () => {
        expect(seconds_to_clock(0)).toBe('0:00')
        expect(seconds_to_clock(65)).toBe('1:05')
        expect(seconds_to_clock(3599.9)).toBe('59:59')
    })

    it('falls back to zero for invalid input', () => {
        expect(seconds_to_clock(-1)).toBe('0:00')
        expect(seconds_to_clock(Number.NaN)).toBe('0:00')
        expect(seconds_to_clock(Number.POSITIVE_INFINITY)).toBe('0:00')
    })
})

describe('seconds_to_clock_hours', () => {
    it('adds an hours field only past one hour', () => {
        expect(seconds_to_clock_hours(59)).toBe('0:59')
        expect(seconds_to_clock_hours(3600)).toBe('1:00:00')
        expect(seconds_to_clock_hours(4023)).toBe('1:07:03')
    })
})
