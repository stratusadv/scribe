import { invoke } from '@tauri-apps/api/core'
import type { InvokeArgs } from '@tauri-apps/api/core'
import type {
    APIEndpoint,
    APIEndpointPurpose,
    Group,
    JobListing,
    JobMeta,
    JobMetaPatch,
    JobSearchHit,
    LineCorrection,
    NotesModels,
    NotesTemplate,
    Person,
    ServiceStatus,
    Settings,
    Transcript,
    TranscriptionResult,
    Waveform,
} from '../types'


async function invoke_void(command: string, call?: InvokeArgs): Promise<void> {
    await invoke(command, call)
}

export const ipc = Object.freeze({
    transcription_remote(audio_path: string, endpoint_id: string, stream_id: string | null) {
        return invoke<TranscriptionResult>('transcription_remote', {
            audio_path,
            endpoint_id,
            stream_id,
        })
    },
    transcript_import(title: string, transcript_text: string) {
        return invoke<TranscriptionResult>('transcript_import', { title, transcript_text })
    },
    recording_start(sample_rate: number) {
        return invoke<string>('recording_start', { sample_rate })
    },
    recording_append(pcm_bytes: Uint8Array) {
        return invoke_void('recording_append', pcm_bytes)
    },
    recording_finish() {
        return invoke<string>('recording_finish')
    },
    recording_discard() {
        return invoke_void('recording_discard')
    },
    recordings_directory_default() {
        return invoke<string>('recordings_directory_default')
    },

    endpoints_list() {
        return invoke<APIEndpoint[]>('endpoints_list')
    },
    service_status() {
        return invoke<ServiceStatus>('service_status')
    },
    service_api_key_set(purpose: APIEndpointPurpose, api_key: string) {
        return invoke_void('service_api_key_set', { purpose, api_key })
    },

    notes_templates_list() {
        return invoke<NotesTemplate[]>('notes_templates_list')
    },
    notes_template_save(template: NotesTemplate) {
        return invoke_void('notes_template_save', { template })
    },
    notes_template_delete(id: string) {
        return invoke_void('notes_template_delete', { id })
    },
    notes_models_list() {
        return invoke<NotesModels>('notes_models_list')
    },
    notes_template_generate(endpoint_id: string, description: string) {
        return invoke<string>('notes_template_generate', { endpoint_id, description })
    },

    people_list() {
        return invoke<Person[]>('people_list')
    },
    person_save(person: Person) {
        return invoke_void('person_save', { person })
    },
    person_remove(id: string) {
        return invoke_void('person_remove', { id })
    },
    groups_list() {
        return invoke<Group[]>('groups_list')
    },
    group_save(group: Group) {
        return invoke_void('group_save', { group })
    },
    group_remove(id: string) {
        return invoke_void('group_remove', { id })
    },
    notes_generate_remote_streaming(
        stream_id: string,
        job_id: string | null,
        transcript_text: string,
        template_id: string,
        endpoint_id: string,
    ) {
        return invoke<string>('notes_generate_remote_streaming', {
            stream_id,
            job_id,
            transcript_text,
            template_id,
            endpoint_id,
        })
    },
    notes_text_rewrite_streaming(
        stream_id: string,
        endpoint_id: string,
        text: string,
        instruction: string,
        template_id: string | null,
    ) {
        return invoke<string>('notes_text_rewrite_streaming', {
            stream_id,
            endpoint_id,
            text,
            instruction,
            template_id,
        })
    },
    transcript_correct_streaming(
        stream_id: string,
        endpoint_id: string,
        lines: string[],
        instruction: string,
    ) {
        return invoke<LineCorrection[]>('transcript_correct_streaming', {
            stream_id,
            endpoint_id,
            lines,
            instruction,
        })
    },
    notes_save(job_id: string, markdown: string) {
        return invoke_void('notes_save', { job_id, markdown })
    },
    notes_load(job_id: string) {
        return invoke<string | null>('notes_load', { job_id })
    },
    notes_export(job_id: string, target_path: string) {
        return invoke<string>('notes_export', { job_id, target_path })
    },
    notes_print(job_id: string) {
        return invoke_void('notes_print', { job_id })
    },

    jobs_list() {
        return invoke<JobListing[]>('jobs_list')
    },
    jobs_search(query: string) {
        return invoke<JobSearchHit[]>('jobs_search', { query })
    },
    job_meta_get(job_id: string) {
        return invoke<JobMeta | null>('job_meta_get', { job_id })
    },
    job_meta_update(job_id: string, patch: JobMetaPatch) {
        return invoke<JobMeta>('job_meta_update', { job_id, patch })
    },
    job_delete(job_id: string) {
        return invoke_void('job_delete', { job_id })
    },
    job_transcript_engines_list(job_id: string) {
        return invoke<string[]>('job_transcript_engines_list', { job_id })
    },
    job_transcript_load(job_id: string, engine_id: string) {
        return invoke<Transcript | null>('job_transcript_load', { job_id, engine_id })
    },
    job_transcript_save(job_id: string, transcript: Transcript) {
        return invoke_void('job_transcript_save', { job_id, transcript })
    },
    job_waveform_get(job_id: string, bar_count: number) {
        return invoke<Waveform | null>('job_waveform_get', { job_id, bar_count })
    },
    job_audio_path_get(job_id: string) {
        return invoke<string | null>('job_audio_path_get', { job_id })
    },

    settings_get() {
        return invoke<Settings>('settings_get')
    },
    settings_update(settings: Settings) {
        return invoke_void('settings_update', { settings })
    },

    task_cancel(stream_id: string) {
        return invoke_void('task_cancel', { stream_id })
    },

    app_relaunch() {
        return invoke_void('app_relaunch')
    },
    markdown_render_html(markdown: string) {
        return invoke<string>('markdown_render_html', { markdown })
    },
})
