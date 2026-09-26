import { ref } from 'vue'
import { ipc } from '../lib/ipc'


interface Capture {
    stream: MediaStream | null
    context: AudioContext | null
    processor: AudioWorkletNode | null
    chunks_pending: Int16Array[]
    samples_pending: number
    samples_total: number
    append_chain: Promise<void>
    append_error: unknown
    flushes_in_flight: number
    flush_timer_id: ReturnType<typeof setInterval> | null
}

const SAMPLE_RATE_PREFERRED = 16000
const FLUSH_INTERVAL_MS = 1000
const FLUSH_BACKLOG_MAX = 30
const CHUNK_BYTES_MAX = 1 << 20
const PCM16_SAMPLE_BYTES = 2
const PCM16_SAMPLE_MAX = 0x7fff
const WORKLET_NAME = 'scribe-recorder'
const MESSAGE_NOT_ALLOWED = 'scribe was not allowed to use the microphone. Allow it and try again.'
const MESSAGE_NOT_FOUND = 'No microphone was found. Plug one in and try again.'
const MESSAGE_NOT_READABLE = 'The microphone is in use by another program.'
const MESSAGE_UNAVAILABLE = 'Microphone recording is not available on this computer.'
const MESSAGE_NOT_RECORDING = 'No recording is in progress.'
const MESSAGE_BACKLOG = 'The recording could not be saved as fast as it was captured.'

const WORKLET_SOURCE = `
class ScribeRecorderProcessor extends AudioWorkletProcessor {
    process(inputs) {
        const channel = inputs[0] && inputs[0][0]
        if (channel) this.port.postMessage(channel.slice())
        return true
    }
}
registerProcessor('${WORKLET_NAME}', ScribeRecorderProcessor)
`

const recording = ref(false)
const elapsed_seconds = ref(0)
const capture = capture_new()


function capture_new(): Capture {
    return {
        stream: null,
        context: null,
        processor: null,
        chunks_pending: [],
        samples_pending: 0,
        samples_total: 0,
        append_chain: Promise.resolve(),
        append_error: null,
        flushes_in_flight: 0,
        flush_timer_id: null,
    }
}

function error_friendly(error: unknown): Error {
    const name = error instanceof DOMException ? error.name : ''

    if (name === 'NotAllowedError') return new Error(MESSAGE_NOT_ALLOWED)
    if (name === 'SecurityError') return new Error(MESSAGE_NOT_ALLOWED)
    if (name === 'NotFoundError') return new Error(MESSAGE_NOT_FOUND)
    if (name === 'OverconstrainedError') return new Error(MESSAGE_NOT_FOUND)
    if (name === 'NotReadableError') return new Error(MESSAGE_NOT_READABLE)

    return error instanceof Error ? error : new Error(String(error))
}

function samples_to_pcm16(input: Float32Array): Int16Array {
    const output = new Int16Array(input.length)

    for (let index = 0; index < input.length; index += 1) {
        const sample = Math.max(-1, Math.min(1, input[index] ?? 0))

        output[index] = Math.round(sample * PCM16_SAMPLE_MAX)
    }

    return output
}

function audio_process(event: MessageEvent) {
    if (!(event.data instanceof Float32Array)) return

    const converted = samples_to_pcm16(event.data)

    capture.chunks_pending.push(converted)
    capture.samples_pending += converted.length
    capture.samples_total += converted.length
}

function pending_bytes_take(): Uint8Array {
    const bytes = new Uint8Array(capture.samples_pending * PCM16_SAMPLE_BYTES)
    const view = new DataView(bytes.buffer)
    let offset = 0

    for (const chunk of capture.chunks_pending) {
        for (const sample of chunk) {
            view.setInt16(offset, sample, true)
            offset += PCM16_SAMPLE_BYTES
        }
    }

    capture.chunks_pending = []
    capture.samples_pending = 0

    return bytes
}

function chunk_append(chunk: Uint8Array) {
    if (capture.flushes_in_flight >= FLUSH_BACKLOG_MAX) {
        capture.append_error ??= new Error(MESSAGE_BACKLOG)

        capture_teardown()

        return
    }

    capture.flushes_in_flight += 1

    capture.append_chain = capture.append_chain
        .then(() => ipc.recording_append(chunk))
        .catch((error: unknown) => {
            capture.append_error ??= error
        })
        .finally(() => {
            capture.flushes_in_flight -= 1
        })
}

function pending_flush() {
    if (capture.samples_pending === 0) return

    const bytes = pending_bytes_take()

    for (let offset = 0; offset < bytes.length; offset += CHUNK_BYTES_MAX) {
        chunk_append(bytes.slice(offset, offset + CHUNK_BYTES_MAX))
    }
}

function flush_tick() {
    pending_flush()

    const rate = capture.context?.sampleRate ?? SAMPLE_RATE_PREFERRED

    elapsed_seconds.value = Math.floor(capture.samples_total / rate)
}

function worklet_url_create(): string {
    const blob = new Blob([WORKLET_SOURCE], { type: 'text/javascript' })

    return URL.createObjectURL(blob)
}

function context_create(): AudioContext {
    try {
        return new AudioContext({ 'sampleRate': SAMPLE_RATE_PREFERRED })
    } catch {
        return new AudioContext()
    }
}

function capture_teardown() {
    if (capture.flush_timer_id !== null) {
        clearInterval(capture.flush_timer_id)
        capture.flush_timer_id = null
    }

    if (capture.processor) {
        capture.processor.port.onmessage = null
        capture.processor.port.close()
        capture.processor.disconnect()
        capture.processor = null
    }

    if (capture.stream) {
        for (const track of capture.stream.getTracks()) track.stop()

        capture.stream = null
    }

    if (capture.context) {
        void capture.context.close().catch((error: unknown) => {
            console.warn('[recorder] audio context close failed', error)
        })

        capture.context = null
    }
}

function state_reset() {
    Object.assign(capture, capture_new())

    elapsed_seconds.value = 0
    recording.value = false
}

async function capture_open() {
    capture.stream = await navigator.mediaDevices.getUserMedia({ audio: true })
    capture.context = context_create()

    await ipc.recording_start(capture.context.sampleRate)
    await capture.context.audioWorklet.addModule(worklet_url_create())

    const source = capture.context.createMediaStreamSource(capture.stream)

    capture.processor = new AudioWorkletNode(capture.context, WORKLET_NAME)
    capture.processor.port.onmessage = audio_process

    source.connect(capture.processor)
    capture.processor.connect(capture.context.destination)
}

async function discard_quietly() {
    try {
        await ipc.recording_discard()
    } catch (error) {
        console.warn('[recorder] discard after failure', error)
    }
}

async function start(): Promise<void> {
    if (recording.value) return
    if (!('mediaDevices' in navigator)) throw new Error(MESSAGE_UNAVAILABLE)

    state_reset()

    try {
        await capture_open()
    } catch (error) {
        capture_teardown()

        await discard_quietly()

        throw error_friendly(error)
    }

    capture.flush_timer_id = setInterval(flush_tick, FLUSH_INTERVAL_MS)
    recording.value = true
}

async function stop(): Promise<string> {
    if (!recording.value) throw new Error(MESSAGE_NOT_RECORDING)

    capture_teardown()
    pending_flush()

    await capture.append_chain

    if (capture.append_error !== null) {
        const failure = capture.append_error

        await discard_quietly()

        state_reset()

        throw error_friendly(failure)
    }

    try {
        return await ipc.recording_finish()
    } finally {
        state_reset()
    }
}

async function discard(): Promise<void> {
    if (!recording.value) return

    capture_teardown()

    await capture.append_chain

    try {
        await ipc.recording_discard()
    } finally {
        state_reset()
    }
}

export function use_recorder() {
    return { recording, elapsed_seconds, start, stop, discard }
}
