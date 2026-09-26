import { defineAsyncComponent, h } from 'vue'


export const rich_editor_async = defineAsyncComponent({
    loader() {
        return import('./RichEditor.vue')
    },
    'loadingComponent': {
        render() {
            return h('div', { class: 'rich-editor' }, [
                h('div', { class: 'rich-editor-toolbar' }, [
                    h('span', { class: 'rich-tb-select invisible' }, ' '),
                ]),
                h('div', { class: 'rich-editor-body' }),
                h('div', { class: 'rich-editor-stats' }, ' '),
            ])
        },
    },
    delay: 0,
})
