import { ref } from 'vue'
import { ipc } from '../lib/ipc'
import type { APIEndpoint, APIEndpointPurpose } from '../types'


const endpoints = ref<APIEndpoint[]>([])
const error_message = ref<string | null>(null)


async function refresh() {
    error_message.value = null

    try {
        endpoints.value = await ipc.endpoints_list()
    } catch (error) {
        error_message.value = String(error)
    }
}

function endpoints_by_purpose(purpose: APIEndpointPurpose): APIEndpoint[] {
    return endpoints.value.filter((endpoint) => endpoint.purpose === purpose)
}

export function use_endpoints() {
    return { endpoints, error_message, refresh, endpoints_by_purpose }
}
