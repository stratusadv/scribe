import { ref } from 'vue'


const online = ref(navigator.onLine)


function online_set(value: boolean) {
    online.value = value
}

window.addEventListener('online', () => {
    online_set(true)
})

window.addEventListener('offline', () => {
    online_set(false)
})

export function use_network() {
    return { online }
}
