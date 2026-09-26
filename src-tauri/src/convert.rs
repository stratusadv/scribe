use crate::error::{AppError, AppResult};
use docmux::html::HTMLRaw;
use docmux::markdown::Options;
use docmux::{Document, Workspace, docx, html, markdown};
use std::sync::{Mutex, MutexGuard};

const OUTPUT_BYTES_MAX: usize = 16 << 20;
const MARKDOWN_BYTES_MAX: usize = OUTPUT_BYTES_MAX >> 2;
const DOCX_MAGIC: &[u8] = b"PK\x03\x04";

const _: () = assert!(MARKDOWN_BYTES_MAX < OUTPUT_BYTES_MAX);

// tigerstyle-ignore: TS005
static DOCUMENT: Mutex<Document> = Mutex::new(Document::EMPTY);
// tigerstyle-ignore: TS005
static OUTPUT: Mutex<[u8; OUTPUT_BYTES_MAX]> = Mutex::new([0; OUTPUT_BYTES_MAX]);
// tigerstyle-ignore: TS005
static WORKSPACE: Mutex<Workspace> = Mutex::new(Workspace::EMPTY);

struct Conversion {
    document: MutexGuard<'static, Document>,
    output: MutexGuard<'static, [u8; OUTPUT_BYTES_MAX]>,
    workspace: MutexGuard<'static, Workspace>,
}

fn conversion_begin(markdown: &str) -> AppResult<Conversion> {
    if markdown.len() > MARKDOWN_BYTES_MAX {
        return Err(AppError::Export(format!(
            "these notes are {} MB, over the {} MB this app can convert",
            markdown.len() >> 20,
            MARKDOWN_BYTES_MAX >> 20
        )));
    }

    let mut document = DOCUMENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut workspace = WORKSPACE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let output = OUTPUT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    debug_assert_eq!(output.len(), OUTPUT_BYTES_MAX);
    debug_assert!(markdown.len() <= MARKDOWN_BYTES_MAX);

    markdown::read(markdown.as_bytes(), Options::GFM, &mut workspace, &mut document)
        .map_err(convert_error)?;

    debug_assert!(document.node_count() >= 1);

    Ok(Conversion { document, output, workspace })
}

fn convert_error(error: docmux::Error) -> AppError {
    AppError::Export(error.to_string())
}

fn output_overflow() -> AppError {
    AppError::Export("converter reported a length past the output buffer".into())
}

fn output_string(output: Vec<u8>) -> AppResult<String> {
    let length = output.len();

    let text = String::from_utf8(output)
        .map_err(|error| AppError::Export(format!("converter produced invalid UTF-8: {error}")))?;

    debug_assert_eq!(text.len(), length);

    Ok(text)
}

pub(crate) fn markdown_to_html(markdown: &str) -> AppResult<String> {
    let mut conversion = conversion_begin(markdown)?;

    let length = html::write(
        &conversion.document,
        HTMLRaw::Escape,
        conversion.output.as_mut_slice(),
    )
    .map_err(convert_error)?;

    let written = conversion.output.get(..length).ok_or_else(output_overflow)?.to_vec();

    debug_assert_eq!(written.len(), length);

    drop(conversion);

    let html = output_string(written)?;

    debug_assert_eq!(html.len(), length);

    Ok(html)
}

pub(crate) fn markdown_to_docx(markdown: &str) -> AppResult<Vec<u8>> {
    let mut conversion = conversion_begin(markdown)?;
    let Conversion { document, output, workspace } = &mut conversion;
    let length = docx::write(document, workspace, output.as_mut_slice()).map_err(convert_error)?;
    let written = output.get(..length).ok_or_else(output_overflow)?;

    debug_assert_eq!(written.len(), length);
    debug_assert!(written.starts_with(DOCX_MAGIC));

    Ok(written.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        "# Title\n\nSome **bold** text and a [link](https://example.com).\n\n",
        "- [x] done\n- [ ] todo\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n"
    );

    #[test]
    fn html_escapes_raw_markup() {
        let html = markdown_to_html("<script>alert(1)</script>\n\n**b**\n").unwrap();

        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<strong>b</strong>"));
    }

    #[test]
    fn docx_is_zip_package() {
        let bytes = markdown_to_docx(SAMPLE).unwrap();

        assert!(bytes.starts_with(DOCX_MAGIC));
        assert!(bytes.len() > 10_000);
    }

    #[test]
    fn markdown_past_the_buffer_limit_is_an_error_not_a_crash() {
        let oversized = "a".repeat(MARKDOWN_BYTES_MAX + 1);

        assert!(markdown_to_html(&oversized).is_err());
        assert!(markdown_to_docx(&oversized).is_err());
    }
}
