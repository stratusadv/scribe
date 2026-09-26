set windows-shell := ["cmd", "/c"]

clear := if os() == "windows" { "cls" } else { "clear" }

manifest := "src-tauri/Cargo.toml"

webview_env_software := if os() == "linux" {
    "GDK_BACKEND=x11 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 " +
    "__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json " +
    "LIBGL_ALWAYS_SOFTWARE=1"
} else { "" }

default:
    @just --list

install:
    bun install

dev:
    {{clear}}
    bun run tauri dev

dev-software:
    {{clear}}
    {{webview_env_software}} bun run tauri dev

build:
    {{clear}}
    bun run tauri build

check:
    {{clear}}
    cargo check --manifest-path {{manifest}}

clippy:
    {{clear}}
    cargo clippy --manifest-path {{manifest}} --all-targets -- -D warnings

typecheck:
    {{clear}}
    bun run typecheck

lint:
    {{clear}}
    bun run lint

lint-fix:
    {{clear}}
    bun run lint:fix

tigerstyle:
    {{clear}}
    tigerstyle check

verify:
    {{clear}}
    cargo clippy --manifest-path {{manifest}} --all-targets -- -D warnings
    cargo test --manifest-path {{manifest}}
    tigerstyle check --ignore TS004
    bun run typecheck
    bun run lint

fmt:
    {{clear}}
    cargo fmt --manifest-path {{manifest}}

fmt-check:
    {{clear}}
    cargo fmt --manifest-path {{manifest}} -- --check

test:
    {{clear}}
    cargo test --manifest-path {{manifest}}

clean:
    {{clear}}
    cargo clean --manifest-path {{manifest}}
