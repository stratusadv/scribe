import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import NoticeBanner from '../../src/components/NoticeBanner.vue'


describe('NoticeBanner', () => {
    it('defaults to an error alert', () => {
        const wrapper = mount(NoticeBanner, { slots: { default: 'Something broke' } })
        const banner = wrapper.get('.banner')

        expect(banner.attributes('data-kind')).toBe('error')
        expect(banner.attributes('role')).toBe('alert')
        expect(wrapper.get('p').text()).toBe('Something broke')
    })

    it('uses the status role for info and warning kinds', () => {
        const info = mount(NoticeBanner, { props: { kind: 'info' } })
        const warning = mount(NoticeBanner, { props: { kind: 'warning' } })

        expect(info.get('.banner').attributes('role')).toBe('status')
        expect(info.get('.banner').attributes('data-kind')).toBe('info')
        expect(warning.get('.banner').attributes('role')).toBe('status')
    })

    it('renders the actions slot before the dismiss button', () => {
        const wrapper = mount(NoticeBanner, {
            slots: { actions: '<button class="retry">Retry</button>' },
        })

        const buttons = wrapper.findAll('.banner-actions button')

        expect(buttons[0]?.classes()).toContain('retry')
        expect(buttons[1]?.attributes('aria-label')).toBe('Dismiss')
    })

    it('emits dismiss when the close button is clicked', async () => {
        const wrapper = mount(NoticeBanner)

        await wrapper.get('.banner-close').trigger('click')

        expect(wrapper.emitted('dismiss')).toHaveLength(1)
    })
})
