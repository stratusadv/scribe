const PATH_SEPARATORS = /[\\/]/
const EXTENSION_SUFFIX = /\.[^.]+$/
const FILE_NAME_CHARS_MAX = 60

const FILE_NAME_CHARS_KEPT = 57
const ELLIPSIS = '…'

export function path_file_name(path: string): string {
    return path.split(PATH_SEPARATORS).pop() ?? path
}

export function path_file_name_short(path: string): string {
    const name = path_file_name(path)

    if (name.length <= FILE_NAME_CHARS_MAX) return name

    return name.slice(0, FILE_NAME_CHARS_KEPT) + ELLIPSIS
}

export function path_file_stem(path: string): string {
    return path_file_name(path).replace(EXTENSION_SUFFIX, '')
}
