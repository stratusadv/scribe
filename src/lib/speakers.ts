import { person_name_full } from '../composables/use_people'
import type { Person, SpeakerLink, Transcript } from './../types'

const LETTER_COUNT = 26
const LETTER_FIRST = 65

const SPEAKER_COLORS = [
    'var(--color-accent)',
    'var(--color-accent-alt)',
    'var(--color-online)',
    'var(--color-warning-content)',
    'var(--color-danger-text)',
]


export interface SpeakerSummary {
    speaker: number
    seconds: number
}

export function speaker_color(speaker: number): string {
    return SPEAKER_COLORS[speaker % SPEAKER_COLORS.length] ?? 'var(--color-accent)'
}

export function speaker_label(speaker: number): string {
    const letter = String.fromCharCode(LETTER_FIRST + (speaker % LETTER_COUNT))
    const round = Math.floor(speaker / LETTER_COUNT)

    return round === 0 ? `Speaker ${letter}` : `Speaker ${letter}${round + 1}`
}

export function speaker_name(
    speaker: number,
    links: SpeakerLink[],
    person_by_id: (id: string) => Person | null,
): string {
    const link = links.find((candidate) => candidate.speaker === speaker)
    const person = link ? person_by_id(link.person_id) : null

    return person ? person_name_full(person) : speaker_label(speaker)
}

export function speakers_summarize(transcript: Transcript): SpeakerSummary[] {
    const summaries = new Map<number, SpeakerSummary>()

    for (const segment of transcript.segments) {
        if (segment.speaker === null) continue

        const seconds = Math.max(0, segment.end_seconds - segment.start_seconds)
        const summary = summaries.get(segment.speaker)

        if (!summary) {
            summaries.set(segment.speaker, { speaker: segment.speaker, seconds })

            continue
        }

        summary.seconds += seconds
    }

    return [...summaries.values()].sort((left, right) => left.speaker - right.speaker)
}
