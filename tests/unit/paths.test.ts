import { describe, expect, it } from 'vitest'
import { path_file_name, path_file_name_short, path_file_stem } from '../../src/lib/paths'


describe('path_file_name', () => {
    it('returns the last segment of a posix path', () => {
        expect(path_file_name('/home/you/recordings/weekly-sync.m4a')).toBe('weekly-sync.m4a')
    })

    it('returns the last segment of a windows path', () => {
        expect(path_file_name('C:\\Users\\you\\Music\\Recording.wav')).toBe('Recording.wav')
    })

    it('returns a bare name unchanged', () => {
        expect(path_file_name('notes.md')).toBe('notes.md')
    })

    it('returns an empty string for a trailing separator', () => {
        expect(path_file_name('/home/you/')).toBe('')
    })
})

describe('path_file_name_short', () => {
    it('keeps a name of sixty characters or fewer intact', () => {
        const name = 'a'.repeat(56) + '.mp3'

        expect(path_file_name_short(`/tmp/${name}`)).toBe(name)
    })

    it('truncates a long name to fifty-seven characters plus an ellipsis', () => {
        const name = 'b'.repeat(80) + '.wav'
        const short = path_file_name_short(`/tmp/${name}`)

        expect(short).toBe('b'.repeat(57) + '…')
        expect(short.length).toBe(58)
    })
})

describe('path_file_stem', () => {
    it('strips the extension from the file name', () => {
        expect(path_file_stem('/home/you/recordings/weekly-sync.m4a')).toBe('weekly-sync')
    })

    it('strips only the final extension', () => {
        expect(path_file_stem('archive.tar.gz')).toBe('archive.tar')
    })

    it('leaves a name without an extension alone', () => {
        expect(path_file_stem('/var/log/syslog')).toBe('syslog')
    })
})
