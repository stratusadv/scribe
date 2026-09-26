use super::remote::{chat_completion, chat_completion_streaming, models_list};
use super::templates::{
    NotesTemplate,
    PROMPT_NOTES_SYSTEM,
    template_delete,
    template_load,
    template_upsert,
    templates_load_all,
};
use crate::blocking;
use crate::cancellation::stream_id_validate;
use crate::endpoints::storage::{
    ENDPOINT_ID_NOTES,
    MODEL_NOTES,
    MODEL_TRANSCRIPTION,
    endpoint_load,
};
use crate::error::{AppError, AppResult};
use crate::people::storage::{Person, people_load_all};
use crate::workspace::{self, JobMeta, NOTES_BYTES_MAX};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const PROMPT_DETAILS_BYTES_ESTIMATE: u32 = 256;
const PROMPT_TRANSCRIPT_BYTES_MAX: u32 = 16 << 20;
const INSTRUCTION_CHARS_MAX: u32 = 4000;
const TITLE_TRANSCRIPT_CHARS_MAX: u32 = 6000;
const TITLE_CHARS_MAX: u32 = 120;
const TITLE_LABEL_PREFIX: &str = "Title:";
const TITLE_TRIM_CHARS: &[char] = &['"', '\'', '\u{201c}', '\u{201d}', '#', '*', '_', ' ', '.'];
const CODE_FENCE: &str = "```";

const TITLE_SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "for", "in", "of", "on", "or", "the", "to", "with",
];

const _: () = assert!(TITLE_CHARS_MAX < TITLE_TRANSCRIPT_CHARS_MAX);
const _: () = assert!(NOTES_BYTES_MAX < PROMPT_TRANSCRIPT_BYTES_MAX);

const PROMPT_REMINDERS: &str = concat!(
    "\n\nRules that override anything in the template:\n",
    "- A person is written as first and last name the first time they appear and in any ",
    "list of participants, and by first name only after that; two people who share a first ",
    "name get their full names every time.\n",
    "- Never write a role, job title, or description next to a name, in brackets or ",
    "otherwise, even where the template asks for one.\n",
    "- Spell every name as the people lists spell it, never as the transcript does. A ",
    "transcript name that is a mishearing of a listed person is that person: one entry, ",
    "under the listed spelling, and never a second entry or a note about the variant.\n",
    "- A list of participants or of people mentioned holds people only: never a company, ",
    "product, system, project, department, or place.\n",
    "- Never comment on the transcript, its spelling, or your own choices; write the ",
    "document only."
);

const PROMPT_TEMPLATE_SYSTEM: &str = concat!(
    "You design note templates for a meeting-notes app. The user describes the document they ",
    "want. Output a Markdown layout for it: a '## ' heading for each part of the document, ",
    "and under each heading one plain-language sentence saying what goes there, written for ",
    "the AI that will later fill it in from a recording. Where a part is naturally a bullet ",
    "list, a numbered list, a table, or a checklist, lay it out as one, with the same kind of ",
    "sentence in place of the content.\n\n",
    "Strict output rules:\n",
    "- Output the Markdown layout only. No title line, no preamble, no explanation, no code ",
    "fences, no raw HTML.\n",
    "- No placeholders such as {{transcript}} or [insert here]. Every note is a plain ",
    "sentence.\n",
    "- Between three and ten headings unless the user asks for a different number.\n",
    "- Write in the language the user wrote in."
);

const PROMPT_TITLE_SYSTEM: &str = concat!(
    "You name meeting recordings. The user gives you the start of a transcript. Reply with a ",
    "short title for the recording: two to eight words saying what the meeting was about, in ",
    "the language of the transcript. Output the title only: no quotes, no full stop at the ",
    "end, no preamble, no explanation."
);

const PROMPT_REWRITE_SYSTEM: &str = concat!(
    "You are a writing assistant inside a note-taking app. Rewrite the user's selected text ",
    "according to their instruction.\n\n",
    "Strict output rules:\n",
    "- Output plain Markdown only. NEVER use raw HTML tags such as <p>, <h1>, <ul>, <li>, ",
    "<br>, etc.\n",
    "- Use Markdown syntax: '## ' for headings, '- ' for bullets, '1. ' for numbered lists, ",
    "'**bold**', '*italic*', '`code`', '> quote'.\n",
    "- Output ONLY the rewritten text. No preamble, no explanation, no surrounding code ",
    "fences.\n",
    "- Preserve any Markdown formatting that already exists in the selected text.\n",
    "- If the selection contains list items, keep the same list structure.\n",
    "- When a template is given, the text is a document written to it or a part of one: keep ",
    "the template's headings, sections, and their order, and never add sections it does not ",
    "have or drop ones it does."
);

const PROMPT_TRANSCRIPT_CORRECT_SYSTEM: &str = concat!(
    "You correct a meeting transcript inside a note-taking app. The user gives the transcript ",
    "as numbered lines and an instruction saying what is wrong with it.\n\n",
    "Strict output rules:\n",
    "- Output only the lines that change, one per line, as '<number>: <corrected line>'.\n",
    "- Keep every word the instruction does not touch exactly as it is: never rephrase, ",
    "summarize, merge, split, reorder, add, or remove lines.\n",
    "- Apply the instruction on every line it applies to, not only the first.\n",
    "- If nothing needs to change, output nothing.\n",
    "- No preamble, no explanation, no code fences."
);

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NotesModels {
    pub(crate) models: Vec<String>,
    pub(crate) model_default: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LineCorrection {
    pub(crate) index: u32,
    pub(crate) text: String,
}

#[tauri::command]
pub(crate) async fn notes_templates_list() -> AppResult<Vec<NotesTemplate>> {
    blocking::run(templates_load_all).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_template_save(template: NotesTemplate) -> AppResult<()> {
    blocking::run(move || template_upsert(template)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_template_delete(id: String) -> AppResult<()> {
    blocking::run(move || template_delete(&id)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_template_generate(
    endpoint_id: String,
    description: String,
) -> AppResult<String> {
    let endpoint = endpoint_load(&endpoint_id)?;
    let description_trimmed = description.trim();

    if description_trimmed.is_empty() {
        return Err(AppError::Config("describe the document first".into()));
    }

    instruction_length_validate(description_trimmed, "the description")?;

    let user_prompt = format!("Document wanted: {description_trimmed}");
    let reply = chat_completion(&endpoint, PROMPT_TEMPLATE_SYSTEM, &user_prompt).await?;
    let layout = code_fence_strip(&reply).to_owned();

    if layout.is_empty() {
        return Err(AppError::API("the AI returned an empty template".into()));
    }

    debug_assert!(!layout.starts_with(CODE_FENCE));
    debug_assert!(!layout.ends_with(CODE_FENCE));

    Ok(layout)
}

#[tauri::command]
pub(crate) async fn notes_models_list() -> AppResult<NotesModels> {
    let endpoint = endpoint_load(ENDPOINT_ID_NOTES)?;

    let models: Vec<String> = models_list(&endpoint)
        .await?
        .into_iter()
        .filter(|model| model != MODEL_TRANSCRIPTION)
        .collect();

    debug_assert!(models.iter().all(|model| model != MODEL_TRANSCRIPTION));

    Ok(NotesModels { models, model_default: MODEL_NOTES.to_owned() })
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_generate_remote_streaming(
    app: tauri::AppHandle,
    stream_id: String,
    job_id: Option<String>,
    transcript_text: String,
    template_id: String,
    endpoint_id: String,
) -> AppResult<String> {
    stream_id_validate(&stream_id)?;
    transcript_text_validate(&transcript_text)?;

    let template = template_load(&template_id)?;
    let endpoint = endpoint_load(&endpoint_id)?;
    let meta = meta_resolve(job_id.as_deref())?;
    let people = people_resolve(meta.as_ref())?;

    let user_prompt =
        prompt_build(&template.instructions, &transcript_text, meta.as_ref(), &people);

    chat_completion_streaming(
        app,
        stream_id,
        &endpoint,
        PROMPT_NOTES_SYSTEM,
        &user_prompt,
    )
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_text_rewrite_streaming(
    app: tauri::AppHandle,
    stream_id: String,
    endpoint_id: String,
    text: String,
    instruction: String,
    template_id: Option<String>,
) -> AppResult<String> {
    stream_id_validate(&stream_id)?;

    let endpoint = endpoint_load(&endpoint_id)?;
    let instruction_trimmed = instruction.trim();

    if instruction_trimmed.is_empty() {
        return Err(AppError::Config("instruction cannot be empty".into()));
    }

    instruction_length_validate(instruction_trimmed, "the instruction")?;

    if text.is_empty() {
        return Err(AppError::Config("selected text cannot be empty".into()));
    }

    if text.len() > NOTES_BYTES_MAX as usize {
        return Err(AppError::Config(format!(
            "the selected text is {} bytes, over the {NOTES_BYTES_MAX} bytes this app rewrites",
            text.len()
        )));
    }

    let template_block = match template_id.as_deref() {
        Some(id) => format!("Template the notes follow:\n{}\n\n", template_load(id)?.instructions),
        None => String::new(),
    };

    let user_prompt = format!(
        "Instruction: {instruction_trimmed}\n\n{template_block}Selected text:\n{text}\n\n\
         Rewritten text:",
    );

    chat_completion_streaming(app, stream_id, &endpoint, PROMPT_REWRITE_SYSTEM, &user_prompt).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn transcript_correct_streaming(
    app: tauri::AppHandle,
    stream_id: String,
    endpoint_id: String,
    lines: Vec<String>,
    instruction: String,
) -> AppResult<Vec<LineCorrection>> {
    stream_id_validate(&stream_id)?;

    let endpoint = endpoint_load(&endpoint_id)?;
    let instruction_trimmed = instruction.trim();

    if instruction_trimmed.is_empty() {
        return Err(AppError::Config("say what to correct first".into()));
    }

    instruction_length_validate(instruction_trimmed, "the instruction")?;

    if lines.is_empty() {
        return Err(AppError::Config("the transcript is empty".into()));
    }

    let Ok(line_count) = u32::try_from(lines.len()) else {
        return Err(AppError::Config("the transcript has too many lines to correct".into()));
    };

    let numbered = lines_number(&lines);

    transcript_text_validate(&numbered)?;

    let user_prompt =
        format!("Instruction: {instruction_trimmed}\n\nTranscript:\n{numbered}\n\nChanged lines:");

    let reply = chat_completion_streaming(
        app,
        stream_id,
        &endpoint,
        PROMPT_TRANSCRIPT_CORRECT_SYSTEM,
        &user_prompt,
    )
    .await?;

    Ok(corrections_parse(&reply, line_count))
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_save(job_id: String, markdown: String) -> AppResult<()> {
    blocking::run(move || workspace::notes_save(&job_id, &markdown)).await
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn notes_load(job_id: String) -> AppResult<Option<String>> {
    blocking::run(move || workspace::notes_load(&job_id)).await
}

pub(crate) async fn title_generate(transcript_text: &str) -> AppResult<String> {
    let mut endpoint = endpoint_load(ENDPOINT_ID_NOTES)?;

    endpoint.reasoning_effort = None;

    if !endpoint.has_api_key {
        return Err(AppError::Config("no notes API key is set".into()));
    }

    let excerpt: String = transcript_text
        .chars()
        .take(TITLE_TRANSCRIPT_CHARS_MAX as usize)
        .collect();

    let excerpt_trimmed = excerpt.trim();

    if excerpt_trimmed.is_empty() {
        return Err(AppError::Config("the transcript is empty".into()));
    }

    let user_prompt = format!("Transcript:\n{excerpt_trimmed}");
    let reply = chat_completion(&endpoint, PROMPT_TITLE_SYSTEM, &user_prompt).await?;
    let title = title_clean(&reply);

    if title.is_empty() {
        return Err(AppError::API("the AI returned no title".into()));
    }

    debug_assert!(title.chars().count() <= TITLE_CHARS_MAX as usize);

    Ok(title)
}

fn title_clean(reply: &str) -> String {
    let line_first = reply.lines().find(|line| !line.trim().is_empty()).unwrap_or("");
    let line_unlabelled = line_first.trim().strip_prefix(TITLE_LABEL_PREFIX).unwrap_or(line_first);
    let stripped = line_unlabelled.trim_matches(TITLE_TRIM_CHARS);
    let capped: String = stripped.chars().take(TITLE_CHARS_MAX as usize).collect();
    let title = title_case(&capped);

    debug_assert!(title.chars().count() <= TITLE_CHARS_MAX as usize);
    debug_assert!(!title.starts_with(TITLE_LABEL_PREFIX));

    title
}

fn title_case(text: &str) -> String {
    let mut title = String::with_capacity(text.len());

    for (index, word) in text.split_whitespace().enumerate() {
        if index > 0 {
            title.push(' ');
        }

        if index > 0 {
            let lower = word.to_lowercase();

            if TITLE_SMALL_WORDS.contains(&lower.as_str()) {
                title.push_str(&lower);

                continue;
            }
        }

        let mut characters = word.chars();

        if let Some(first) = characters.next() {
            title.extend(first.to_uppercase());
            title.push_str(characters.as_str());
        }
    }

    debug_assert_eq!(title.split_whitespace().count(), text.split_whitespace().count());

    title
}

fn lines_number(lines: &[String]) -> String {
    let mut numbered = String::new();

    for (position, line) in lines.iter().enumerate() {
        let _ = writeln!(numbered, "{}: {}", position + 1, line.trim());
    }

    debug_assert_eq!(numbered.lines().count(), lines.len());

    numbered
}

fn corrections_parse(reply: &str, line_count: u32) -> Vec<LineCorrection> {
    let mut by_index: BTreeMap<u32, String> = BTreeMap::new();

    for line in code_fence_strip(reply).lines() {
        let Some((number, text)) = line.trim().split_once(':') else {
            continue;
        };

        let Ok(number) = number.trim().parse::<u32>() else {
            continue;
        };

        if number == 0 {
            continue;
        }

        if number > line_count {
            continue;
        }

        let text_trimmed = text.trim();

        if text_trimmed.is_empty() {
            continue;
        }

        let _previous = by_index.insert(number - 1, text_trimmed.to_owned());
    }

    let corrections: Vec<LineCorrection> = by_index
        .into_iter()
        .map(|(index, text)| LineCorrection { index, text })
        .collect();

    debug_assert!(corrections.iter().all(|correction| correction.index < line_count));
    debug_assert!(corrections.is_sorted_by_key(|correction| correction.index));

    corrections
}

fn code_fence_strip(reply: &str) -> &str {
    let trimmed = reply.trim();

    let Some(after_open) = trimmed.strip_prefix(CODE_FENCE) else {
        return trimmed;
    };

    let body = after_open.split_once('\n').map_or("", |(_, rest)| rest);
    let stripped = body.trim_end().strip_suffix(CODE_FENCE).unwrap_or(body).trim();

    debug_assert!(stripped.len() < reply.len());
    debug_assert!(!stripped.starts_with(CODE_FENCE));

    stripped
}

fn instruction_length_validate(text: &str, label: &str) -> AppResult<()> {
    debug_assert!(!label.is_empty());

    if text.chars().count() > INSTRUCTION_CHARS_MAX as usize {
        return Err(AppError::Config(format!(
            "{label} is too long; keep it under {INSTRUCTION_CHARS_MAX} characters"
        )));
    }

    Ok(())
}

fn transcript_text_validate(transcript_text: &str) -> AppResult<()> {
    if transcript_text.trim().is_empty() {
        return Err(AppError::Config("the transcript is empty".into()));
    }

    if transcript_text.len() > PROMPT_TRANSCRIPT_BYTES_MAX as usize {
        return Err(AppError::Config(format!(
            "the transcript is {} bytes, over the {PROMPT_TRANSCRIPT_BYTES_MAX} bytes this app \
             sends for notes",
            transcript_text.len()
        )));
    }

    Ok(())
}

fn prompt_build(
    instructions: &str,
    transcript: &str,
    meta: Option<&JobMeta>,
    people: &[Person],
) -> String {
    let title = meta.and_then(|meta| meta.title.as_deref()).unwrap_or("");
    let project = meta.and_then(|meta| meta.project.as_deref()).unwrap_or("");
    let people_present = meta.map(|meta| people_present_describe(meta, people)).unwrap_or_default();

    let people_mentioned = meta
        .map(|meta| people_describe(&meta.person_ids_mentioned, people))
        .unwrap_or_default();

    let tags_joined = meta.map(|meta| meta.tags.join(", ")).unwrap_or_default();

    let recorded_on = meta
        .map(|meta| meta.recorded_at_unix.unwrap_or(meta.created_at_unix))
        .and_then(|unix| chrono::DateTime::from_timestamp(unix, 0))
        .map(|recorded| recorded.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    let details = [
        ("Title: ", title),
        ("Project: ", project),
        ("People present: ", people_present.as_str()),
        ("People mentioned but not present: ", people_mentioned.as_str()),
        ("Tags: ", tags_joined.as_str()),
        ("Recorded on: ", recorded_on.as_str()),
    ];

    let mut prompt = String::with_capacity(
        instructions.len() + transcript.len() + PROMPT_DETAILS_BYTES_ESTIMATE as usize,
    );

    prompt.push_str("Template:\n");
    prompt.push_str(instructions.trim());
    prompt.push_str("\n\n");

    let details_start = prompt.len();

    for (label, value) in details {
        if !value.is_empty() {
            prompt.push_str(label);
            prompt.push_str(value);
            prompt.push('\n');
        }
    }

    if prompt.len() > details_start {
        prompt.push('\n');
    }

    prompt.push_str("Transcript:\n");
    prompt.push_str(transcript);
    prompt.push_str(PROMPT_REMINDERS);

    debug_assert!(prompt.ends_with(PROMPT_REMINDERS));
    debug_assert!(prompt.len() > instructions.len() + transcript.len());

    prompt
}

fn people_present_describe(meta: &JobMeta, people: &[Person]) -> String {
    let described = people_describe(&meta.person_ids, people);

    if described.is_empty() {
        return meta.attendees.join(", ");
    }

    described
}

fn people_describe(ids: &[String], people: &[Person]) -> String {
    let described: Vec<String> = ids
        .iter()
        .filter_map(|id| people.iter().find(|person| &person.id == id))
        .map(person_describe)
        .collect();

    debug_assert!(described.len() <= ids.len());

    described.join("; ")
}

fn person_describe(person: &Person) -> String {
    let mut text = person.name_full();

    if !person.role.is_empty() {
        text.push_str(" (");
        text.push_str(&person.role);
        text.push(')');
    }

    if !person.description.is_empty() {
        text.push_str(": ");
        text.push_str(&person.description);
    }

    debug_assert!(text.starts_with(&person.name_first));

    text
}

fn people_resolve(meta: Option<&JobMeta>) -> AppResult<Vec<Person>> {
    let Some(meta) = meta else {
        return Ok(Vec::new());
    };

    if meta.person_ids.is_empty() {
        if meta.person_ids_mentioned.is_empty() {
            return Ok(Vec::new());
        }
    }

    people_load_all()
}

fn meta_resolve(job_id: Option<&str>) -> AppResult<Option<JobMeta>> {
    job_id.map_or_else(|| Ok(None), workspace::meta_load)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrections_keep_only_numbered_lines_inside_the_transcript() {
        let reply = concat!(
            "```\n",
            "Here are the changes:\n",
            "2: Dana said Kalymma, not Kalima.\n",
            "9: out of range\n",
            "0: also out of range\n",
            "3:   \n",
            "1: first\n",
            "1: first, corrected twice\n",
            "```"
        );

        let corrections = corrections_parse(reply, 4);

        assert_eq!(corrections.len(), 2);
        assert_eq!(corrections[0].index, 0);
        assert_eq!(corrections[0].text, "first, corrected twice");
        assert_eq!(corrections[1].index, 1);
        assert_eq!(corrections[1].text, "Dana said Kalymma, not Kalima.");
        assert!(corrections_parse("", 4).is_empty());
    }

    #[test]
    fn numbered_lines_start_at_one_and_carry_every_line() {
        let lines = vec![" a ".to_owned(), "b".to_owned()];

        assert_eq!(lines_number(&lines), "1: a\n2: b\n");
    }

    #[test]
    fn every_meeting_detail_is_sent_with_the_instructions() {
        let meta = JobMeta {
            id: "abc".to_owned(),
            source_path: "/tmp/a.wav".to_owned(),
            source_size_bytes: 1,
            created_at_unix: 1_737_460_800,
            recorded_at_unix: Some(1_736_596_800),
            label: None,
            title: Some("Kickoff".to_owned()),
            attendees: vec!["Dana".to_owned(), "Sam".to_owned()],
            person_ids: vec!["p-dana".to_owned(), "p-gone".to_owned()],
            person_ids_mentioned: vec!["p-sam".to_owned()],
            project: Some("Website".to_owned()),
            tags: vec!["weekly".to_owned()],
            favourite: false,
        };

        let people = vec![
            Person {
                id: "p-dana".to_owned(),
                name_first: "Jane".to_owned(),
                name_last: "Doe".to_owned(),
                role: "project manager".to_owned(),
                description: "runs the delivery side".to_owned(),
            },
            Person {
                id: "p-sam".to_owned(),
                name_first: "Sam".to_owned(),
                name_last: String::new(),
                role: String::new(),
                description: String::new(),
            },
        ];

        let prompt = prompt_build(" Write a summary. ", "hello", Some(&meta), &people);

        assert_eq!(
            prompt,
            concat!(
                "Template:\nWrite a summary.\n\n",
                "Title: Kickoff\n",
                "Project: Website\n",
                "People present: Jane Doe (project manager): runs the delivery side\n",
                "People mentioned but not present: Sam\n",
                "Tags: weekly\n",
                "Recorded on: 2025-01-11\n",
                "\nTranscript:\nhello",
            )
            .to_owned()
                + PROMPT_REMINDERS,
        );
    }

    #[test]
    fn a_recording_without_metadata_sends_only_instructions_and_transcript() {
        let prompt = prompt_build("Write a summary.", "hello", None, &[]);

        assert_eq!(
            prompt,
            "Template:\nWrite a summary.\n\nTranscript:\nhello".to_owned() + PROMPT_REMINDERS
        );
    }

    #[test]
    fn a_title_loses_its_quotes_label_and_full_stop() {
        assert_eq!(
            title_clean("Title: \"Office Move Plan\".\n\nExtra"),
            "Office Move Plan"
        );

        assert_eq!(title_clean("## Spring planting\n"), "Spring Planting");
        assert_eq!(title_clean("\n   \n"), "");
    }

    #[test]
    fn a_title_is_set_in_title_case_with_small_words_kept_low() {
        assert_eq!(title_case("the state of the office move"), "The State of the Office Move");
        assert_eq!(title_case("Q3 budget  review with IT"), "Q3 Budget Review with IT");
    }

    #[test]
    fn a_fenced_template_is_unwrapped() {
        assert_eq!(code_fence_strip("```markdown\n## A\nnote\n```\n"), "## A\nnote");
        assert_eq!(code_fence_strip("## A\nnote"), "## A\nnote");
    }

    #[test]
    fn an_empty_or_oversized_transcript_is_refused_before_a_prompt_is_built() {
        let oversized = "a".repeat(PROMPT_TRANSCRIPT_BYTES_MAX as usize + 1);

        assert!(transcript_text_validate("  \n").is_err());
        assert!(transcript_text_validate("hello").is_ok());
        assert!(transcript_text_validate(&oversized).is_err());
    }

    #[test]
    fn an_instruction_past_its_length_is_refused() {
        assert!(instruction_length_validate("shorten this", "the instruction").is_ok());

        let long = "a".repeat(INSTRUCTION_CHARS_MAX as usize + 1);

        assert!(instruction_length_validate(&long, "the instruction").is_err());
    }
}
