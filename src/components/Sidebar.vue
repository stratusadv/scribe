<script setup lang="ts">
import { defineComponent, h, ref } from 'vue'
import type { PropType } from 'vue'
import { use_pipeline } from '../composables/use_pipeline'
import TasksPanel from './TasksPanel.vue'
import type { View } from '../composables/use_pipeline'


type IconName =
    | 'jobs'

    | 'templates'
    | 'people'
    | 'settings'

interface NavItem {
    id: View
    label: string
    icon: IconName
}

const COLLAPSED_STORAGE_KEY = 'scribe.sidebar.collapsed'
const { view_current, view_reopen, pipeline_reset } = use_pipeline()

const collapsed = ref<boolean>(
    typeof localStorage !== 'undefined'
        && localStorage.getItem(COLLAPSED_STORAGE_KEY) === '1',
)

const workflow_items: NavItem[] = [
    { id: 'jobs', label: 'Home', icon: 'jobs' },
    { id: 'templates', label: 'Templates', icon: 'templates' },
    { id: 'people', label: 'People', icon: 'people' },
]

const ICON_PATHS: Record<IconName, string[]> = {
    jobs: ['M3 11l9-7 9 7', 'M5 10v10h14V10', 'M10 20v-6h4v6'],
    templates: [
        'M4.5 4.5h15v15h-15Z',
        'M4.5 9.75h15',
        'M9.75 9.75v9.75',
    ],
    people: [
        'M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0Z',
        'M4.5 20.25a7.5 7.5 0 0 1 15 0v.75h-15v-.75Z',
    ],
    settings: [
        'M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.65.83a6.748 6.748 0 0 1 1.124.65c.317.234.715.298 1.067.156l1.213-.485a1.125 1.125 0 0 1 1.37.49l1.296 2.247a1.125 1.125 0 0 1-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992.014.418.014.838 0 1.255-.008.378.137.75.43.991l1.004.828a1.125 1.125 0 0 1 .26 1.431l-1.298 2.247a1.125 1.125 0 0 1-1.37.491l-1.213-.486c-.352-.141-.75-.077-1.066.158a6.81 6.81 0 0 1-1.125.65c-.337.144-.587.456-.65.83l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.65-.83a6.749 6.749 0 0 1-1.125-.65c-.317-.235-.715-.299-1.067-.158l-1.213.486a1.125 1.125 0 0 1-1.369-.49l-1.297-2.247a1.125 1.125 0 0 1 .26-1.432l1.004-.827c.292-.241.437-.613.43-.992a7.07 7.07 0 0 1 0-1.255c.007-.378-.138-.75-.43-.991l-1.004-.828a1.125 1.125 0 0 1-.26-1.431l1.297-2.247a1.125 1.125 0 0 1 1.37-.491l1.212.486c.352.142.75.078 1.067-.157.347-.26.722-.479 1.125-.65.337-.144.587-.456.65-.83l.214-1.28Z',
        'M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z',
    ],
}

const SidebarIcon = defineComponent({
    name: 'SidebarIcon',
    props: { name: { type: String as PropType<IconName>, required: true } },
    setup(icon_props) {
        return () => h(
            'svg',
            {
                class: 'sidebar-nav-icon',
                viewBox: '0 0 24 24',
                fill: 'none',
                stroke: 'currentColor',
                'stroke-width': 1.5,
                'stroke-linecap': 'round',
                'stroke-linejoin': 'round',
                'aria-hidden': 'true',
            },
            ICON_PATHS[icon_props.name].map((path_data) => h('path', { d: path_data })),
        )
    },
})


function collapse_toggle() {
    collapsed.value = !collapsed.value

    if (typeof localStorage !== 'undefined') {
        localStorage.setItem(COLLAPSED_STORAGE_KEY, collapsed.value ? '1' : '0')
    }
}

function nav_button_class(item_id: View): string {
    return view_current.value === item_id ? 'sidebar-nav-item-active' : 'sidebar-nav-item'
}
</script>

<template>
    <aside
        class="sidebar shrink-0 border-r flex flex-col"
        :data-collapsed="collapsed ? 'true' : 'false'"
    >
        <div class="sidebar-header">
            <button
                v-if="!collapsed"
                class="sidebar-brand"
                title="Reset and go to recordings"
                @click="pipeline_reset"
            >
                <span>scribe</span>
            </button>
            <button
                class="sidebar-icon-btn"
                :title="collapsed ? 'Expand sidebar' : 'Collapse sidebar'"
                @click="collapse_toggle"
            >
                <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                >
                    <path v-if="collapsed" d="M9 6l6 6-6 6" />
                    <path v-else d="M15 6l-6 6 6 6" />
                </svg>
            </button>
        </div>

        <nav class="flex-1 px-2.5 pb-3 overflow-y-auto">
            <ul class="flex flex-col gap-0.5">
                <li v-for="item in workflow_items" :key="item.id">
                    <button
                        :class="nav_button_class(item.id)"
                        :title="collapsed ? item.label : undefined"
                        @click="view_reopen(item.id)"
                    >
                        <SidebarIcon :name="item.icon" />
                        <span v-if="!collapsed">{{ item.label }}</span>
                    </button>
                </li>
            </ul>
        </nav>

        <TasksPanel v-if="!collapsed" />

        <div class="sidebar-divider sidebar-footer-actions border-t">
            <button
                :class="nav_button_class('settings')"
                :title="collapsed ? 'Settings' : undefined"
                @click="view_reopen('settings')"
            >
                <SidebarIcon name="settings" />
                <span v-if="!collapsed">Settings</span>
            </button>
        </div>
    </aside>
</template>
