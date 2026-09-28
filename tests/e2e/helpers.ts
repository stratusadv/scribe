import { expect } from '@playwright/test'
import type { Locator, Page } from '@playwright/test'


export const RECORDING_TITLED = 'Weekly platform sync'
export const RECORDING_UNTITLED = 'discovery-call.wav'
export const STREAM_TIMEOUT_MS = 30_000

export async function app_open(page: Page) {
    await page.goto('/')
    await expect(page.locator('.app-shell[data-ready="true"]')).toBeVisible()
}

export function dialog(page: Page, title: string): Locator {
    return page.getByRole('dialog').filter({ has: page.getByRole('heading', { name: title }) })
}

export function editor(page: Page): Locator {
    return page.locator('.rich-editor-content .ProseMirror')
}

export async function editor_content_set(page: Page, markdown: string) {
    await page.getByRole('button', { name: 'Markdown', exact: true }).click()
    await page.locator('.rich-editor-source').fill(markdown)
    await page.getByRole('button', { name: 'Rich editor' }).click()
    await expect(editor(page)).toBeVisible()
}

export async function editor_line_select(page: Page, text: string) {
    await editor(page).getByText(text, { exact: true }).click()
    await page.keyboard.press('End')
    await page.keyboard.press('Shift+Home')
}

export async function nav_to(page: Page, label: string) {
    await page.locator('aside').getByRole('button', { name: label, exact: true }).click()
    await expect(page.getByRole('heading', { name: label })).toBeVisible()
}

export async function recording_open(page: Page, title: string) {
    await tiles(page).filter({ hasText: title }).click()
    await expect(step_current(page)).toBeVisible()
}

export async function row_menu_pick(page: Page, label: string, item: string) {
    await page.getByRole('button', { name: `Options for ${label}` }).click()
    await page.getByRole('menuitem', { name: item }).click()
}

export function setting(page: Page, heading: string): Locator {
    return page
        .locator('section > div > div')
        .filter({ has: page.getByRole('heading', { name: heading }) })
}

export function step_current(page: Page): Locator {
    return page.locator('.step-bar-item[aria-current="step"]')
}

export function tiles(page: Page): Locator {
    return page.locator('.recording-tile:not(.recording-tile-new)')
}

export function transcript_rows(page: Page): Locator {
    return page.locator('.transcript-pane-row')
}

export async function transcript_step_open(page: Page) {
    await recording_open(page, RECORDING_UNTITLED)
    await page.getByRole('button', { name: 'Transcript', exact: true }).click()
    await expect(transcript_rows(page)).toHaveCount(3)
}
