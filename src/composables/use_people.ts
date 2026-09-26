import { computed, ref } from 'vue'
import { ipc } from '../lib/ipc'
import type { Group, Person } from '../types'


const people = ref<Person[]>([])
const groups = ref<Group[]>([])
const error_message = ref<string | null>(null)

const people_sorted = computed(() =>
    [...people.value].sort((left, right) =>
        left.name_last.localeCompare(right.name_last)
            || left.name_first.localeCompare(right.name_first),
    ),
)

const groups_sorted = computed(() =>
    [...groups.value].sort((left, right) => left.name.localeCompare(right.name)),
)


export function person_name_full(person: Person): string {
    return `${person.name_first} ${person.name_last}`.trim()
}

function person_by_id(id: string): Person | null {
    return people.value.find((person) => person.id === id) ?? null
}

function group_members(group: Group): Person[] {
    return group.person_ids
        .map(person_by_id)
        .filter((person): person is Person => person !== null)
}

async function refresh() {
    error_message.value = null

    try {
        const [people_loaded, groups_loaded] = await Promise.all([
            ipc.people_list(),
            ipc.groups_list(),
        ])

        people.value = people_loaded
        groups.value = groups_loaded
    } catch (error) {
        error_message.value = String(error)
    }
}

async function save(person: Person) {
    error_message.value = null

    try {
        await ipc.person_save(person)
        await refresh()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function remove(id: string) {
    error_message.value = null

    try {
        await ipc.person_remove(id)
        await refresh()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function group_save(group: Group) {
    error_message.value = null

    try {
        await ipc.group_save(group)
        await refresh()
    } catch (error) {
        error_message.value = String(error)
    }
}

async function group_remove(id: string) {
    error_message.value = null

    try {
        await ipc.group_remove(id)
        await refresh()
    } catch (error) {
        error_message.value = String(error)
    }
}

export function use_people() {
    return {
        people,
        people_sorted,
        groups,
        groups_sorted,
        error_message,
        person_by_id,
        group_members,
        refresh,
        save,
        remove,
        group_save,
        group_remove,
    }
}
