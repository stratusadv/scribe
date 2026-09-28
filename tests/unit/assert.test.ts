import { describe, expect, it } from 'vitest'
import { assert } from '../../src/lib/assert'


describe('assert', () => {
    it('returns without throwing when the condition holds', () => {
        expect(() => {
            assert(true, 'never raised')
        }).not.toThrow()
    })

    it('throws an Error carrying the message when the condition fails', () => {
        expect(() => {
            assert(false, 'the waveform has too many bars')
        }).toThrow(new Error('the waveform has too many bars'))
    })
})
