import { beforeEach, describe, expect, it } from 'vitest'
import { use_dialog } from '../../src/composables/use_dialog'


const dialog = use_dialog()

beforeEach(() => {
    dialog.close(false)
})

describe('confirm', () => {
    it('opens a dangerous warning dialog with default labels', () => {
        void dialog.confirm('Delete it?')

        expect(dialog.current.value).toMatchObject({
            title: 'scribe',
            message: 'Delete it?',
            kind: 'warning',
            ok_label: 'OK',
            cancel_label: 'Cancel',
            danger: true,
        })

        expect(dialog.current.value?.id).toMatch(/^[0-9a-f-]{36}$/)
    })

    it('applies the given options and is not dangerous for info', () => {
        void dialog.confirm('Continue?', {
            title: 'Heads up',
            kind: 'info',
            ok_label: 'Yes',
            cancel_label: 'No',
        })

        expect(dialog.current.value).toMatchObject({
            title: 'Heads up',
            kind: 'info',
            ok_label: 'Yes',
            cancel_label: 'No',
            danger: false,
        })
    })

    it('marks an error confirm as dangerous', () => {
        void dialog.confirm('Really?', { kind: 'error' })

        expect(dialog.current.value?.danger).toBe(true)
    })

    it('resolves true when closed with true and clears the state', async () => {
        const pending = dialog.confirm('Go?')

        dialog.close(true)

        expect(await pending).toBe(true)
        expect(dialog.current.value).toBeNull()
    })

    it('resolves false when closed with false', async () => {
        const pending = dialog.confirm('Go?')

        dialog.close(false)

        expect(await pending).toBe(false)
    })

    it('resolves a superseded dialog with false when a new one opens', async () => {
        const first = dialog.confirm('First')
        const second = dialog.confirm('Second')

        expect(await first).toBe(false)
        expect(dialog.current.value?.message).toBe('Second')

        dialog.close(true)

        expect(await second).toBe(true)
    })
})

describe('message', () => {
    it('opens an info dialog without a cancel button', () => {
        void dialog.message('Saved.')

        expect(dialog.current.value).toMatchObject({
            title: 'scribe',
            message: 'Saved.',
            kind: 'info',
            ok_label: 'OK',
            cancel_label: null,
            danger: false,
        })
    })

    it('resolves once closed regardless of the value', async () => {
        const options = { title: 'Done', kind: 'warning', ok_label: 'Fine' } as const
        const pending = dialog.message('Saved.', options)

        expect(dialog.current.value?.title).toBe('Done')
        expect(dialog.current.value?.kind).toBe('warning')
        expect(dialog.current.value?.ok_label).toBe('Fine')

        dialog.close(false)

        await expect(pending).resolves.toBeUndefined()
    })
})

describe('close', () => {
    it('is a no-op when nothing is open', () => {
        expect(() => {
            dialog.close(true)
        }).not.toThrow()
        expect(dialog.current.value).toBeNull()
    })
})
