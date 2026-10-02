import type { Group, JobListing, JobMeta, Person, Transcript } from '../../src/types'


export function group_build(patch: Partial<Group> = {}): Group {
    return { id: 'group-team', name: 'Product team', person_ids: [], ...patch }
}

export function job_listing_build(patch: Partial<JobListing> = {}): JobListing {
    return {
        ...job_meta_build(),
        has_transcript: false,
        has_notes: false,
        duration_seconds: null,
        ...patch,
    }
}

export function job_meta_build(patch: Partial<JobMeta> = {}): JobMeta {
    return {
        id: 'job-1',
        source_path: '/home/you/recordings/weekly-sync.m4a',
        source_size_bytes: 1024,
        created_at_unix: 1_775_000_000,
        recorded_at_unix: null,
        label: null,
        title: null,
        attendees: [],
        person_ids: [],
        person_ids_mentioned: [],
        project: null,
        tags: [],
        favourite: false,
        speaker_links: [],
        ...patch,
    }
}

export function person_build(patch: Partial<Person> = {}): Person {
    return {
        id: 'person-john',
        name_first: 'John',
        name_last: 'Doe',
        role: 'Developer',
        description: '',
        ...patch,
    }
}

export function transcript_build(lines: string[]): Transcript {
    const segments = lines.map((text, index) => ({
        text,
        start_seconds: index * 10,
        end_seconds: (index + 1) * 10,
        speaker: null,
    }))

    return { text: lines.join(' '), segments }
}
