import { expect, test } from '@playwright/test'
import {
    RECORDING_TITLED,
    RECORDING_UNTITLED,
    app_open,
    dialog,
    nav_to,
    recording_open,
    step_current,
    tiles,
    transcript_rows,
} from './helpers'
import type { Page } from '@playwright/test'


function people_field(page: Page, label: string) {
    return page.locator('.step-field').filter({ has: page.getByText(label, { exact: true }) })
}

function chips(page: Page, label: string) {
    return people_field(page, label).locator('.tag-chip')
}

test.beforeEach(async ({ page }) => {
    await app_open(page)
})

test('the title autosaves and shows on the home tile', async ({ page }) => {
    await recording_open(page, RECORDING_UNTITLED)

    const title = page.getByLabel('Title')

    await expect(title).toHaveValue('')

    await title.fill('Discovery call')
    await nav_to(page, 'Home')

    await expect(tiles(page).locator('.recording-tile-title')).toContainText(['Discovery call'])
})

test('the existing meeting shows its people, template, and title', async ({ page }) => {
    await recording_open(page, RECORDING_TITLED)
    await page.getByRole('button', { name: 'Meeting' }).click()

    await expect(page.getByLabel('Title')).toHaveValue(RECORDING_TITLED)
    await expect(page.getByLabel('Template')).toHaveValue('Meeting notes')
    await expect(chips(page, 'In the meeting')).toHaveText([
        'John Doe (Developer)',
        'Jane Doe (Product owner)',
    ])
    await expect(chips(page, 'Mentioned')).toHaveText(['Sam Smith (Client)'])
    await expect(page.getByText('These names are from before People existed')).toBeHidden()
})

test('the picker adds people who were present, including a whole group', async ({ page }) => {
    await recording_open(page, RECORDING_UNTITLED)
    await people_field(page, 'In the meeting').getByRole('button', { name: '+ Add' }).click()

    const picker = dialog(page, 'Who was in the meeting?')

    await expect(picker).toContainText('Tick each person who was there.')
    await expect(picker.getByRole('checkbox')).toHaveCount(4)

    const group = picker.getByRole('checkbox', { name: 'Product team' })

    await group.check()

    await expect(group).toBeChecked()
    await expect(picker.getByRole('checkbox', { name: 'John Doe (Developer)' })).toBeChecked()
    await expect(picker.getByRole('checkbox', { name: 'Jane Doe (Product owner)' })).toBeChecked()
    await expect(picker.getByRole('checkbox', { name: 'Sam Smith (Client)' })).not.toBeChecked()

    await picker.getByRole('checkbox', { name: 'Sam Smith (Client)' }).check()
    await picker.getByRole('button', { name: 'Done' }).click()

    await expect(picker).toBeHidden()
    await expect(chips(page, 'In the meeting')).toHaveText([
        'John Doe (Developer)',
        'Jane Doe (Product owner)',
        'Sam Smith (Client)',
    ])

    await chips(page, 'In the meeting').getByRole('button', { name: 'Remove Sam Smith' }).click()

    await expect(chips(page, 'In the meeting')).toHaveCount(2)

    await nav_to(page, 'Home')

    const tile = tiles(page).filter({ hasText: RECORDING_UNTITLED })

    await expect(tile.locator('.recording-tile-people')).toHaveText('Jane Doe, John Doe')
})

test('unticking a group removes every member', async ({ page }) => {
    await recording_open(page, RECORDING_TITLED)
    await page.getByRole('button', { name: 'Meeting' }).click()
    await people_field(page, 'In the meeting').getByRole('button', { name: '+ Add' }).click()

    const picker = dialog(page, 'Who was in the meeting?')
    const group = picker.getByRole('checkbox', { name: 'Product team' })

    await expect(group).toBeChecked()

    await group.uncheck()
    await picker.getByRole('button', { name: 'Done' }).click()

    await expect(chips(page, 'In the meeting')).toHaveCount(0)
})

test('the picker filters by name and role', async ({ page }) => {
    await recording_open(page, RECORDING_UNTITLED)
    await people_field(page, 'In the meeting').getByRole('button', { name: '+ Add' }).click()

    const picker = dialog(page, 'Who was in the meeting?')
    const filter = picker.getByRole('searchbox', { name: 'Find a person' })

    await filter.fill('jane')

    await expect(picker.getByRole('checkbox')).toHaveCount(1)
    await expect(picker.getByRole('checkbox', { name: 'Jane Doe (Product owner)' })).toBeVisible()

    await filter.fill('client')

    await expect(picker.getByRole('checkbox', { name: 'Sam Smith (Client)' })).toBeVisible()

    await filter.fill('nobody here')

    await expect(picker.getByText('Nobody matches that name.')).toBeVisible()
})

test('the picker creates a new person and ticks them', async ({ page }) => {
    await recording_open(page, RECORDING_UNTITLED)
    await people_field(page, 'In the meeting').getByRole('button', { name: '+ Add' }).click()

    const picker = dialog(page, 'Who was in the meeting?')

    await picker.getByRole('button', { name: 'New person' }).click()
    await picker.getByPlaceholder('e.g. John').fill('Ada')
    await picker.getByPlaceholder('e.g. Doe').fill('Lovelace')
    await picker.getByPlaceholder('e.g. project manager, accountant, sales rep').fill('Engineer')
    await picker.getByRole('button', { name: 'Save' }).click()

    await expect(picker.getByRole('checkbox', { name: 'Ada Lovelace (Engineer)' })).toBeChecked()

    await picker.getByRole('button', { name: 'Done' }).click()

    await expect(chips(page, 'In the meeting')).toHaveText(['Ada Lovelace (Engineer)'])
})

test('the picker adds mentioned people', async ({ page }) => {
    await recording_open(page, RECORDING_UNTITLED)
    await people_field(page, 'Mentioned').getByRole('button', { name: '+ Add' }).click()

    const picker = dialog(page, 'Who was talked about?')

    await expect(picker).toContainText('Tick people who came up in conversation')

    await picker.getByRole('checkbox', { name: 'Sam Smith (Client)' }).check()
    await picker.getByRole('button', { name: 'Close' }).click()

    await expect(picker).toBeHidden()
    await expect(chips(page, 'Mentioned')).toHaveText(['Sam Smith (Client)'])
    await expect(chips(page, 'In the meeting')).toHaveCount(0)

    await chips(page, 'Mentioned').getByRole('button', { name: 'Remove Sam Smith' }).click()

    await expect(chips(page, 'Mentioned')).toHaveCount(0)
})

test('a speaker can be assigned to a saved person and the transcript shows the name', async ({ page }) => {
    await recording_open(page, RECORDING_TITLED)
    await page.getByRole('button', { name: 'Meeting' }).click()

    const speakers = people_field(page, 'Speakers')

    await expect(speakers.locator('.meeting-speaker-label')).toHaveText(['Speaker A', 'Speaker B'])
    await expect(chips(page, 'In the meeting')).toHaveCount(2)

    await page.getByLabel('Who is Speaker A').fill('sam')
    await page.getByRole('option', { name: 'Sam Smith (Client)' }).click()

    await expect(chips(page, 'In the meeting')).toHaveCount(3)
    await expect(chips(page, 'In the meeting').last()).toContainText('Sam Smith')

    await page.getByRole('button', { name: 'Transcript', exact: true }).click()

    await expect(transcript_rows(page).first().locator('.transcript-pane-row-speaker'))
        .toHaveText('Sam Smith')
    await expect(transcript_rows(page).nth(1).locator('.transcript-pane-row-speaker'))
        .toHaveText('Speaker B')
})

test('the template select changes the template and Next opens Notes', async ({ page }) => {
    await recording_open(page, RECORDING_UNTITLED)

    const template = page.getByLabel('Template')

    await expect(template).toHaveValue('Meeting notes')

    await template.fill('stand')
    await page.getByRole('option', { name: 'Stand-up digest' }).click()

    await expect(template).toHaveValue('Stand-up digest')

    await page.getByRole('button', { name: 'Next' }).click()

    await expect(step_current(page)).toHaveText('Notes')

    await page.getByRole('button', { name: 'Back' }).click()

    await expect(step_current(page)).toHaveText('Meeting')
    await expect(template).toHaveValue('Stand-up digest')
})

test('Back returns to the Transcript step', async ({ page }) => {
    await recording_open(page, RECORDING_UNTITLED)
    await page.getByRole('button', { name: 'Back' }).click()

    await expect(step_current(page)).toHaveText('Transcript')
})
