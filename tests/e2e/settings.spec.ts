import { expect, test } from '@playwright/test'
import { app_open, dialog, nav_to, setting } from './helpers'
import type { Page } from '@playwright/test'


const RECORDINGS_DIRECTORY_DEFAULT = 'C:\\Users\\mock\\Music\\scribe'
const RECORDINGS_DIRECTORY_PICKED = '/home/you/Music/meetings'
const KEY_PLACEHOLDER_SET = '•'.repeat(24)
const KEY_PLACEHOLDER_EMPTY = 'Paste your key'

async function ai_tab_open(page: Page) {
    await page.getByRole('tab', { name: 'AI' }).click()
    await expect(page.getByRole('tab', { name: 'AI' })).toHaveAttribute('aria-selected', 'true')
}

async function key_field_check(page: Page, label: string) {
    const field = setting(page, label)
    const input = field.locator('input[type="password"]')
    const remove = field.getByRole('button', { name: 'Remove' })
    const save = field.getByRole('button', { name: 'Save' })

    await expect(input).toHaveAttribute('placeholder', KEY_PLACEHOLDER_SET)
    await expect(remove).toBeEnabled()
    await expect(save).toBeDisabled()

    await input.fill('fresh-key')

    await expect(save).toBeEnabled()

    await save.click()

    await expect(input).toHaveValue('')
    await expect(input).toHaveAttribute('placeholder', KEY_PLACEHOLDER_SET)

    await remove.click()

    const confirm = dialog(page, 'Remove key')

    await expect(confirm)
        .toContainText('Remove your key? scribe will not work until you add another one.')

    await confirm.getByRole('button', { name: 'Cancel' }).click()

    await expect(remove).toBeEnabled()

    await remove.click()
    await confirm.getByRole('button', { name: 'Remove' }).click()

    await expect(input).toHaveAttribute('placeholder', KEY_PLACEHOLDER_EMPTY)
    await expect(remove).toBeDisabled()

    await input.fill('another-key')
    await input.press('Enter')

    await expect(input).toHaveAttribute('placeholder', KEY_PLACEHOLDER_SET)
    await expect(remove).toBeEnabled()
}

test.beforeEach(async ({ page }) => {
    await app_open(page)
    await nav_to(page, 'Settings')
})

test('the theme select updates the html class', async ({ page }) => {
    const theme = page.getByRole('combobox').filter({ hasText: 'Match my computer' })
    const html = page.locator('html')

    await expect(theme).toHaveValue('dark')
    await expect(html).toHaveClass(/theme-dark/)

    await theme.selectOption('light')

    await expect(html).toHaveClass(/theme-light/)
    await expect(html).not.toHaveClass(/theme-dark/)

    await theme.selectOption('dark')

    await expect(html).toHaveClass(/theme-dark/)
    await expect(html).not.toHaveClass(/theme-light/)

    await theme.selectOption('system')

    await expect(theme).toHaveValue('system')
    await expect(html).toHaveClass(/theme-(light|dark)/)
})

test('the palette swatches update data-palette', async ({ page }) => {
    const html = page.locator('html')
    const graphite = page.getByRole('radio', { name: 'Graphite' })
    const nord = page.getByRole('radio', { name: 'Nord' })

    await expect(html).toHaveAttribute('data-palette', 'graphite')
    await expect(graphite).toHaveAttribute('aria-checked', 'true')

    await nord.click()

    await expect(html).toHaveAttribute('data-palette', 'nord')
    await expect(nord).toHaveAttribute('aria-checked', 'true')
    await expect(graphite).toHaveAttribute('aria-checked', 'false')

    await page.getByRole('radio', { name: 'Dracula' }).click()

    await expect(html).toHaveAttribute('data-palette', 'dracula')
})

test('the default template select feeds the Templates page', async ({ page }) => {
    const template = page.getByRole('combobox').filter({ hasText: 'Ask each time' })

    await expect(template).toHaveValue('default-meeting-notes')

    await template.selectOption({ label: 'Stand-up digest' })

    await expect(template).toHaveValue('default-standup')

    await nav_to(page, 'Templates')

    await expect(page.locator('li').filter({ hasText: 'Stand-up digest' }).locator('.pill-active'))
        .toHaveText('Default')

    await nav_to(page, 'Settings')
    await template.selectOption({ label: 'Ask each time' })

    await expect(template).toHaveValue('')

    await nav_to(page, 'Templates')

    await expect(page.locator('.pill-active')).toHaveCount(0)

    await nav_to(page, 'Home')
    await page.locator('.recording-tile').filter({ hasText: 'discovery-call.wav' }).click()

    await expect(page.getByLabel('Template')).toHaveValue('')
    await expect(page.getByLabel('Template')).toHaveAttribute('placeholder', 'Choose a template')
    await expect(page.getByRole('button', { name: 'Next' })).toBeDisabled()
})

test('the recordings folder can be chosen and reset', async ({ page }) => {
    const field = setting(page, 'Recordings')
    const path = field.getByRole('textbox')
    const reset = field.getByRole('button', { name: 'Use default' })

    await expect(path).toHaveValue(RECORDINGS_DIRECTORY_DEFAULT)
    await expect(reset).toHaveCount(0)

    await field.getByRole('button', { name: 'Choose folder' }).click()

    await expect(path).toHaveValue(RECORDINGS_DIRECTORY_PICKED)

    await reset.click()

    await expect(path).toHaveValue(RECORDINGS_DIRECTORY_DEFAULT)
    await expect(reset).toHaveCount(0)
})

test('About shows the version and checks for updates', async ({ page }) => {
    const about = setting(page, 'About')

    await expect(about).toContainText('scribe 0.1.0-browser-mock')

    await about.getByRole('button', { name: 'Check for updates' }).click()

    await expect(about).toContainText('scribe is up to date.')
})

test('the API address can be set and removed', async ({ page }) => {
    await ai_tab_open(page)

    const field = setting(page, 'API address')
    const input = field.getByRole('textbox')
    const remove = field.getByRole('button', { name: 'Remove' })
    const save = field.getByRole('button', { name: 'Save' })

    await expect(input).toHaveValue('')
    await expect(remove).toBeDisabled()
    await expect(save).toBeDisabled()

    await input.fill('https://api.example.test')

    await expect(save).toBeEnabled()

    await save.click()

    await expect(remove).toBeEnabled()

    await nav_to(page, 'Home')
    await nav_to(page, 'Settings')
    await ai_tab_open(page)

    await expect(input).toHaveValue('https://api.example.test')

    await remove.click()

    const confirm = dialog(page, 'Remove address')

    await expect(confirm)
        .toContainText('Remove your address? scribe will go back to the built-in one.')

    await confirm.getByRole('button', { name: 'Cancel' }).click()

    await expect(input).toHaveValue('https://api.example.test')

    await remove.click()
    await confirm.getByRole('button', { name: 'Remove' }).click()

    await expect(input).toHaveValue('')
    await expect(remove).toBeDisabled()
})

test('the transcription key can be set and removed', async ({ page }) => {
    await ai_tab_open(page)
    await key_field_check(page, 'Transcription key')
})

test('the notes key can be set and removed', async ({ page }) => {
    await ai_tab_open(page)
    await key_field_check(page, 'Notes key')
})

test('removing the notes key blocks note generation until it is back', async ({ page }) => {
    await ai_tab_open(page)

    const field = setting(page, 'Notes key')

    await field.getByRole('button', { name: 'Remove' }).click()
    await dialog(page, 'Remove key').getByRole('button', { name: 'Remove' }).click()

    await expect(field.getByRole('button', { name: 'Remove' })).toBeDisabled()

    await nav_to(page, 'Home')
    await page.locator('.recording-tile').filter({ hasText: 'discovery-call.wav' }).click()
    await page.getByRole('button', { name: 'Next' }).click()

    await expect(page.getByText('scribe needs the API address and keys set up')).toBeVisible()
    await expect(page.getByRole('button', { name: 'Generate notes' })).toBeDisabled()
})

test('the notes model and thinking selects persist', async ({ page }) => {
    await ai_tab_open(page)

    const model = setting(page, 'Model').getByRole('combobox')
    const thinking = setting(page, 'Thinking').getByRole('combobox')

    await expect(model).toHaveValue('')
    await expect(model.locator('option').first()).toHaveText('Default (stratus.thinking)')
    await expect(model.locator('option')).toHaveCount(4)
    await expect(thinking).toHaveValue('')

    await model.selectOption('stratus.turbo')
    await thinking.selectOption('low')

    await nav_to(page, 'Home')
    await nav_to(page, 'Settings')
    await ai_tab_open(page)

    await expect(model).toHaveValue('stratus.turbo')
    await expect(thinking).toHaveValue('low')

    await model.selectOption('')
    await thinking.selectOption('xhigh')

    await expect(model).toHaveValue('')
    await expect(thinking).toHaveValue('xhigh')
})

test('the tabs switch between General and AI', async ({ page }) => {
    const general = page.getByRole('tab', { name: 'General' })

    await expect(general).toHaveAttribute('aria-selected', 'true')
    await expect(setting(page, 'Appearance')).toBeVisible()
    await expect(setting(page, 'API address')).toHaveCount(0)

    await ai_tab_open(page)

    await expect(setting(page, 'API address')).toBeVisible()
    await expect(setting(page, 'Appearance')).toHaveCount(0)

    await page.getByRole('tab', { name: 'General' }).click()

    await expect(setting(page, 'Appearance')).toBeVisible()
})
