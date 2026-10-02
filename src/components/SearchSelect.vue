<script setup lang="ts">
import { computed, ref, useId } from 'vue'


export interface SearchSelectOption {
    value: string
    text: string
}

const props = defineProps<{
    label: string
    options: SearchSelectOption[]
    color?: string
    value: string | null
    empty_text?: string
    id?: string
    placeholder?: string
}>()

const emit = defineEmits<{
    'update:value': [value: string]
}>()

const open = ref(false)
const query = ref('')
const index_active = ref(0)
const input_ref = ref<HTMLInputElement | null>(null)
const list_id = useId()

const option_empty = computed<SearchSelectOption | null>(() =>
    props.empty_text === undefined ? null : { value: '', text: props.empty_text },
)

const text_selected = computed(
    () => props.options.find((option) => option.value === props.value)?.text ?? '',
)

const options_shown = computed<SearchSelectOption[]>(() => {
    const needle = query.value.trim().toLowerCase()

    if (needle.length === 0) {
        return option_empty.value ? [option_empty.value, ...props.options] : props.options
    }

    return props.options.filter((option) => option.text.toLowerCase().includes(needle))
})

function list_open() {
    open.value = true
    query.value = ''
    index_active.value = 0
}

function list_close() {
    open.value = false
    query.value = ''
}

function option_pick(option: SearchSelectOption) {
    emit('update:value', option.value)
    list_close()
    input_ref.value?.blur()
}

function input_handler(event: Event) {
    if (!(event.target instanceof HTMLInputElement)) return

    query.value = event.target.value
    index_active.value = 0
    open.value = true
}

function active_move(step: number) {
    if (!open.value) {
        list_open()

        return
    }

    const count = options_shown.value.length

    if (count === 0) return

    index_active.value = (index_active.value + step + count) % count
}

function active_pick() {
    if (!open.value) return

    const option = options_shown.value[index_active.value]

    if (!option) return

    option_pick(option)
}
</script>

<template>
    <div
        class="search-select"
        :style="props.color ? { '--search-select-color': props.color } : undefined"
    >
        <input
            :id="props.id"
            ref="input_ref"
            type="text"
            class="input"
            role="combobox"
            autocomplete="off"
            :placeholder="props.placeholder ?? props.empty_text"
            :aria-label="props.label"
            :aria-expanded="open ? 'true' : 'false'"
            :aria-controls="list_id"
            :value="open ? query : text_selected"
            @focus="list_open"
            @blur="list_close"
            @input="input_handler"
            @keydown.down.prevent="active_move(1)"
            @keydown.up.prevent="active_move(-1)"
            @keydown.enter.prevent="active_pick"
            @keydown.esc.stop="input_ref?.blur()"
        />
        <ul v-if="open" :id="list_id" class="menu-popover search-select-list" role="listbox">
            <li
                v-for="(option, index) in options_shown"
                :key="option.value"
                class="menu-item"
                role="option"
                :aria-selected="option.value === (props.value ?? '') ? 'true' : 'false'"
                :data-active="index === index_active ? 'true' : 'false'"
                @mousedown.prevent="option_pick(option)"
                @mouseenter="index_active = index"
            >
                <span class="truncate">{{ option.text }}</span>
            </li>
            <li v-if="options_shown.length === 0" class="meta px-2.5 py-1.5">Nothing matches.</li>
        </ul>
    </div>
</template>
