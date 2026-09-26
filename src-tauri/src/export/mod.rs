use crate::blocking;
use crate::convert;
use crate::error::{AppError, AppResult};
use crate::workspace;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const PRINT_FILE_NAME: &str = "print.html";
const PRINT_WINDOW_LABEL: &str = "print";
const PRINT_WINDOW_WIDTH: f64 = 960.0;
const PRINT_WINDOW_HEIGHT: f64 = 1100.0;
const PAGE_BYTES_ESTIMATE: u32 = 1024;
const EXPORT_EXTENSION_DOCX: &str = "docx";
const EXPORT_EXTENSION_MARKDOWN: &str = "md";
const TITLE_DEFAULT: &str = "Notes";

fn notes_markdown_load(job_id: &str) -> AppResult<String> {
    debug_assert!(!job_id.is_empty());

    let markdown = workspace::notes_load(job_id)?
        .ok_or_else(|| AppError::Export(format!("no notes found for job {job_id}")))?;

    if markdown.trim().is_empty() {
        return Err(AppError::Export("notes are empty".into()));
    }

    let stripped = timestamps_strip(&markdown);

    debug_assert!(stripped.len() <= markdown.len());

    Ok(stripped)
}

fn timestamp_match_at(chars: &[char], index: usize) -> Option<usize> {
    const INNER_LENGTH_MIN: u32 = 2;
    const INNER_LENGTH_MAX: u32 = 24;

    const _: () = assert!(INNER_LENGTH_MIN < INNER_LENGTH_MAX);

    debug_assert!(index < chars.len());

    let closer = match *chars.get(index)? {
        '[' => ']',
        '(' => ')',
        _ => return None,
    };

    let inner_start = index + 1;
    let mut has_colon = false;
    let mut has_digit_or_marker = false;
    let inner = chars.iter().enumerate().skip(inner_start).take(INNER_LENGTH_MAX as usize + 1);

    for (offset, character) in inner {
        if *character == closer {
            if !has_colon {
                return None;
            }

            if !has_digit_or_marker {
                return None;
            }

            let inner_length = offset - inner_start;

            if !(INNER_LENGTH_MIN as usize..=INNER_LENGTH_MAX as usize).contains(&inner_length) {
                return None;
            }

            debug_assert!(offset + 1 > index);
            debug_assert!(offset < chars.len());

            return Some(offset + 1);
        }

        match *character {
            ':' => has_colon = true,
            'm' | 's' | 'h' | 'M' | 'S' | 'H' => has_digit_or_marker = true,
            '-' | '\u{2013}' | '\u{2014}' | ',' | ' ' | '.' => {}
            digit if digit.is_ascii_digit() => has_digit_or_marker = true,
            _ => return None,
        }
    }

    None
}

fn timestamps_strip(markdown: &str) -> String {
    let chars: Vec<char> = markdown.chars().collect();
    let mut without_timestamps = String::with_capacity(markdown.len());
    let mut index = 0;

    while let Some(&character) = chars.get(index) {
        if let Some(end_index) = timestamp_match_at(&chars, index) {
            debug_assert!(end_index > index);

            without_timestamps.push(' ');
            index = end_index;

            continue;
        }

        without_timestamps.push(character);
        index += 1;
    }

    debug_assert_eq!(index, chars.len());

    let collapsed = timestamps_strip_spaces_collapse(&without_timestamps);
    let tightened = timestamps_strip_punctuation_tighten(&collapsed);
    let trimmed = timestamps_strip_line_ends(&tightened);

    debug_assert!(trimmed.len() <= without_timestamps.len());

    trimmed
}

fn timestamps_strip_spaces_collapse(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut space_horizontal_open = false;

    for character in text.chars() {
        if matches!(character, ' ' | '\t') {
            if !space_horizontal_open {
                collapsed.push(' ');
            }

            space_horizontal_open = true;

            continue;
        }

        collapsed.push(character);
        space_horizontal_open = false;
    }

    debug_assert!(collapsed.len() <= text.len());
    debug_assert!(!collapsed.contains("  "));

    collapsed
}

fn timestamps_strip_punctuation_tighten(text: &str) -> String {
    const PUNCTUATION: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}'];
    let mut tightened = String::with_capacity(text.len());
    let mut remaining = text.chars().peekable();
    let mut previous = '\n';

    for _character_index in 0..text.len() {
        let Some(character) = remaining.next() else {
            break;
        };

        let space_before_punctuation = character == ' '
            && remaining
                .peek()
                .is_some_and(|next| PUNCTUATION.contains(next));

        if space_before_punctuation {
            let task_marker_space = previous == '[' && remaining.peek() == Some(&']');

            if task_marker_space {
                tightened.push(character);
            }
        } else {
            tightened.push(character);
        }

        previous = character;
    }

    debug_assert!(tightened.len() <= text.len());
    debug_assert!(!tightened.contains(" ."));
    debug_assert!(!tightened.contains(" ,"));

    tightened
}

fn timestamps_strip_line_ends(text: &str) -> String {
    let mut trimmed = String::with_capacity(text.len());

    for line in text.split_inclusive('\n') {
        let (body, newline) = line.strip_suffix('\n').map_or((line, ""), |rest| (rest, "\n"));

        trimmed.push_str(body.trim_end_matches([' ', '\t']));
        trimmed.push_str(newline);
    }

    debug_assert!(trimmed.len() <= text.len());
    debug_assert_eq!(trimmed.matches('\n').count(), text.matches('\n').count());

    trimmed
}

fn notes_export_write_blocking(job_id: &str, target_path: &Path) -> AppResult<()> {
    let extension = target_path
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let markdown = notes_markdown_load(job_id)?;

    let bytes = match extension.as_str() {
        EXPORT_EXTENSION_DOCX => convert::markdown_to_docx(&markdown)?,
        EXPORT_EXTENSION_MARKDOWN => markdown.into_bytes(),
        _ => {
            return Err(AppError::Export(format!(
                "target path must end with .{EXPORT_EXTENSION_DOCX} or .{EXPORT_EXTENSION_MARKDOWN}"
            )));
        }
    };

    debug_assert!(!bytes.is_empty());

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(target_path, &bytes)?;

    debug_assert!(target_path.exists());

    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_export(job_id: String, target_path: String) -> AppResult<String> {
    let target_path_owned = PathBuf::from(&target_path);

    blocking::run(move || {
        notes_export_write_blocking(&job_id, &target_path_owned)?;

        Ok(target_path_owned.to_string_lossy().into_owned())
    })
    .await
}

const PRINT_PAGE_STYLE: &str = r#"
:root { color-scheme: light; }
body { margin: 0; padding: 2rem; max-width: 52rem; margin-inline: auto; color: #1a1a1a;
  font: 11pt/1.5 system-ui, "Segoe UI", Roboto, sans-serif; background: #fff; }
.banner { background: #fff5d6; border: 1px solid #e0c060; border-radius: 6px; padding: 0.75rem 1rem;
  margin-bottom: 1.5rem; font-size: 10pt; }
h1 { font-size: 20pt; margin: 1.2em 0 0.4em; } h2 { font-size: 16pt; margin: 1.2em 0 0.4em; }
h3 { font-size: 13pt; margin: 1em 0 0.3em; } h4, h5, h6 { font-size: 11pt; margin: 1em 0 0.3em; }
p, ul, ol, pre, table, blockquote { margin: 0 0 0.8em; }
code { font-family: Consolas, "Courier New", monospace; font-size: 10pt;
  background: #f2f2f2; padding: 0 0.2em; }
pre { background: #f2f2f2; padding: 0.75em; overflow-x: auto; white-space: pre-wrap; }
pre code { background: none; padding: 0; }
blockquote { border-left: 3px solid #bfbfbf; margin-left: 0; padding-left: 1em; color: #404040; }
table { border-collapse: collapse; width: 100%; }
th, td { border: 1px solid #bfbfbf; padding: 0.3em 0.6em; text-align: left; vertical-align: top; }
th { background: #f2f2f2; }
hr { border: 0; border-top: 1px solid #bfbfbf; margin: 1.2em 0; }
a { color: #0563c1; }
input[type=checkbox] { vertical-align: middle; margin-right: 0.4em; }
ul:has(> li > input[type=checkbox]) { list-style: none; padding-left: 0; }
@page { margin: 2cm; }
@media print { .banner { display: none; } body { padding: 0; max-width: none; } }
"#;

fn print_page_html(title: &str, body_html: &str) -> String {
    let capacity = body_html.len() + PRINT_PAGE_STYLE.len() + PAGE_BYTES_ESTIMATE as usize;
    let mut page = String::with_capacity(capacity);

    page.push_str("<!doctype html><html><head><meta charset=\"utf-8\"><title>");
    page.push_str(&html_text_escape(title));
    page.push_str("</title><style>");
    page.push_str(PRINT_PAGE_STYLE);
    page.push_str("</style></head><body><div class=\"banner\">");
    page.push_str("The print dialog opens with this page. Choose ");
    page.push_str("\u{201c}Save as PDF\u{201d} as the printer to save a PDF file, ");
    page.push_str("then close this window.");
    page.push_str("</div>");
    page.push_str(body_html);
    page.push_str("</body></html>");

    debug_assert!(page.len() > body_html.len());
    debug_assert!(page.ends_with("</html>"));

    page
}

fn html_text_escape(text: &str) -> String {
    let escaped = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");

    debug_assert!(!escaped.contains('<'));
    debug_assert!(!escaped.contains('>'));

    escaped
}

fn notes_print_page_write_blocking(job_id: &str) -> AppResult<PathBuf> {
    let markdown = notes_markdown_load(job_id)?;
    let body_html = convert::markdown_to_html(&markdown)?;

    let title = workspace::meta_load(job_id)?
        .and_then(|meta| meta.title)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| TITLE_DEFAULT.to_owned());

    let page = print_page_html(&title, &body_html);
    let path = workspace::job_directory(job_id)?.join(PRINT_FILE_NAME);

    debug_assert!(page.starts_with("<!doctype html>"));

    workspace::atomic_write(&path, page.as_bytes())?;

    debug_assert!(path.ends_with(PRINT_FILE_NAME));
    debug_assert!(path.is_absolute());

    Ok(path)
}

fn asset_url(path: &Path) -> String {
    let mut encoded = String::with_capacity(path.as_os_str().len() * 3);

    for byte in path.to_string_lossy().bytes() {
        let plain = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');

        if plain {
            encoded.push(char::from(byte));

            continue;
        }

        let _ = write!(encoded, "%{byte:02X}");
    }

    debug_assert!(encoded.len() >= path.to_string_lossy().len());
    debug_assert!(!encoded.contains(' '));

    // tigerstyle-ignore: TS035
    if cfg!(windows) {
        format!("http://asset.localhost/{encoded}")
    } else {
        format!("asset://localhost/{encoded}")
    }
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_print(app: AppHandle, job_id: String) -> AppResult<()> {
    let page_path = blocking::run(move || notes_print_page_write_blocking(&job_id)).await?;

    debug_assert!(page_path.is_absolute());

    let url = tauri::Url::parse(&asset_url(&page_path))
        .map_err(|error| AppError::Export(format!("print page url: {error}")))?;

    debug_assert!(!url.scheme().is_empty());

    if let Some(existing) = app.get_webview_window(PRINT_WINDOW_LABEL) {
        existing
            .close()
            .map_err(|error| AppError::Export(format!("could not close print window: {error}")))?;
    }

    WebviewWindowBuilder::new(&app, PRINT_WINDOW_LABEL, WebviewUrl::External(url))
        .title("Print notes")
        .inner_size(PRINT_WINDOW_WIDTH, PRINT_WINDOW_HEIGHT)
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                if let Err(error) = window.print() {
                    tracing::warn!("print dialog could not be opened: {}", error);
                }
            }
        })
        .build()
        .map_err(|error| AppError::Export(format!("could not open print window: {error}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_markers_are_removed_and_spacing_is_tidied() {
        let stripped =
            timestamps_strip("Budget approved [1:23] by Dana (12:04).\nNext item [0:07] .\n");

        assert_eq!(stripped, "Budget approved by Dana.\nNext item.\n");
    }

    #[test]
    fn task_markers_keep_their_space() {
        assert_eq!(timestamps_strip("- [ ] todo [0:07]\n- [x] done\n"), "- [ ] todo\n- [x] done\n");
    }

    #[test]
    fn text_without_markers_survives_untouched() {
        assert_eq!(timestamps_strip("Plain [note] here.\n"), "Plain [note] here.\n");
    }

    #[test]
    fn a_bracket_that_runs_on_too_long_is_not_a_marker() {
        let long = format!("[{}:00] end", "1".repeat(40));

        assert_eq!(timestamps_strip(&long), long);
        let deep = "[1:00:00:00:00:00:00:00:00:00:00] x";

        assert_eq!(timestamps_strip(deep), deep);
    }

    #[test]
    fn markup_characters_in_a_title_are_escaped() {
        assert_eq!(html_text_escape("a <b> & c"), "a &lt;b&gt; &amp; c");
    }
}
