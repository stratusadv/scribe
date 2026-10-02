import { describe, expect, it } from 'vitest'
import { speaker_label, speakers_summarize } from '../../src/lib/speakers'
import { transcript_build } from './fixtures'


describe('speaker_label', () => {
    it('letters the first twenty-six and numbers the rounds after', () => {
        expect(speaker_label(0)).toBe('Speaker A')
        expect(speaker_label(25)).toBe('Speaker Z')
        expect(speaker_label(26)).toBe('Speaker A2')
    })
})

describe('speakers_summarize', () => {
    it('sums talk time per speaker', () => {
        const transcript = transcript_build(['short', 'other voice', 'a much longer line here', 'tail'])
        const speakers = [1, 0, 1, null]

        transcript.segments = transcript.segments.map((segment, index) => ({
            ...segment,
            speaker: speakers[index] ?? null,
            end_seconds: index === 2 ? 45 : segment.end_seconds,
        }))

        const summaries = speakers_summarize(transcript)

        expect(summaries).toEqual([
            { speaker: 0, seconds: 10 },
            { speaker: 1, seconds: 35 },
        ])
        expect(speakers_summarize(transcript_build(['no speakers']))).toEqual([])
    })
})
