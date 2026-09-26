import { createApp } from 'vue'
import App from './App.vue'
import { theme_apply_to_html, theme_resolve, theme_storage_read } from './lib/theme'
import './styles.css'


theme_apply_to_html(theme_resolve(theme_storage_read()))

if (import.meta.env.DEV) {
    if (!('__TAURI_INTERNALS__' in window)) {
        const { tauri_browser_mock_install } = await import('./lib/tauri_browser_mock')

        tauri_browser_mock_install()
    }
}

createApp(App).mount('#app')
