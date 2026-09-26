export const AUDIO_EXTENSIONS = Object.freeze([
    'mp3',
    'wav',
    'm4a',
    'flac',
    'ogg',
    'oga',
    'opus',
    'aac',
    'aiff',
    'aif',
    'wma',
    'amr',
    'ac3',
    'caf',
    'mka',
    'spx',
])

export const VIDEO_EXTENSIONS = Object.freeze([
    'mp4',
    'm4v',
    'mkv',
    'mov',
    'avi',
    'webm',
    'wmv',
    'flv',
    '3gp',
    '3g2',
    'mpg',
    'mpeg',
    'ts',
    'mts',
    'm2ts',
    'vob',
])

export const MEDIA_EXTENSIONS = Object.freeze([...AUDIO_EXTENSIONS, ...VIDEO_EXTENSIONS])

export function path_is_media(path: string): boolean {
    const dot = path.lastIndexOf('.')

    if (dot < 0) return false

    const extension = path.slice(dot + 1).toLowerCase()

    return MEDIA_EXTENSIONS.includes(extension)
}
