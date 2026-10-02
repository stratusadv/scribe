use crate::error::{AppError, AppResult};
use crate::workspace;
use serde::{Deserialize, Serialize};

const PEOPLE_FILE: &str = "people.json";
const GROUPS_FILE: &str = "groups.json";
const PEOPLE_FILE_BYTES_MAX: u32 = 32 << 20;
const GROUPS_FILE_BYTES_MAX: u32 = 16 << 20;
const PEOPLE_COUNT_MAX: u32 = 1024;
const GROUP_COUNT_MAX: u32 = 256;
const PERSON_NAME_CHARS_MAX: u32 = 120;
const PERSON_ROLE_CHARS_MAX: u32 = 120;
const PERSON_DESCRIPTION_CHARS_MAX: u32 = 2000;
const GROUP_NAME_CHARS_MAX: u32 = 120;
const PERSON_ID_BYTES_ESTIMATE: u32 = 48;

const PERSON_BYTES_MAX: u32 =
    (PERSON_NAME_CHARS_MAX * 2 + PERSON_ROLE_CHARS_MAX + PERSON_DESCRIPTION_CHARS_MAX) * 8;

const GROUP_BYTES_MAX: u32 =
    PEOPLE_COUNT_MAX * PERSON_ID_BYTES_ESTIMATE + GROUP_NAME_CHARS_MAX * 8;

const _: () = assert!(GROUP_COUNT_MAX <= PEOPLE_COUNT_MAX);

const _: () = assert!(PEOPLE_FILE_BYTES_MAX >= PEOPLE_COUNT_MAX * PERSON_BYTES_MAX);
const _: () = assert!(GROUPS_FILE_BYTES_MAX >= GROUP_COUNT_MAX * GROUP_BYTES_MAX);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Person {
    pub(crate) id: String,
    #[serde(alias = "first_name")]
    pub(crate) name_first: String,
    #[serde(alias = "last_name")]
    pub(crate) name_last: String,
    #[serde(default)]
    pub(crate) role: String,
    #[serde(default)]
    pub(crate) description: String,
}

impl Person {
    pub(crate) fn name_full(&self) -> String {
        let full = format!("{} {}", self.name_first, self.name_last);
        let trimmed = full.trim().to_owned();

        debug_assert!(trimmed.len() <= full.len());

        trimmed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Group {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) person_ids: Vec<String>,
}

pub(crate) fn people_load_all() -> AppResult<Vec<Person>> {
    let path = workspace::root_file_path(PEOPLE_FILE)?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = workspace::file_read_bounded(&path, PEOPLE_FILE_BYTES_MAX)?;

    let people: Vec<Person> = serde_json::from_str(&raw).map_err(|error| {
        AppError::Config(format!("the saved people could not be read: {error}"))
    })?;

    people_count_validate(&people)?;

    Ok(people)
}

fn people_count_validate(people: &[Person]) -> AppResult<()> {
    if people.len() > PEOPLE_COUNT_MAX as usize {
        return Err(AppError::Config(format!(
            "there are {} people, over the {PEOPLE_COUNT_MAX} this app keeps",
            people.len()
        )));
    }

    Ok(())
}

fn people_save_all(people: &[Person]) -> AppResult<()> {
    people_count_validate(people)?;

    let path = workspace::root_file_path(PEOPLE_FILE)?;
    let encoded = serde_json::to_string_pretty(people)?;

    debug_assert!(encoded.len() <= PEOPLE_FILE_BYTES_MAX as usize);

    workspace::atomic_write(&path, encoded.as_bytes())
}

pub(crate) fn person_upsert(person: &Person) -> AppResult<()> {
    let person = person_normalize(person);

    person_validate(&person)?;

    let id = person.id.clone();
    let mut people = people_load_all()?;

    match people.iter_mut().find(|existing| existing.id == person.id) {
        Some(existing) => *existing = person,
        None => people.push(person),
    }

    debug_assert_eq!(people.iter().filter(|existing| existing.id == id).count(), 1);

    people_save_all(&people)
}

pub(crate) fn person_delete(id: &str) -> AppResult<()> {
    let mut people = people_load_all()?;
    let people_count_before = people.len();

    people.retain(|person| person.id != id);

    debug_assert!(people.len() <= people_count_before);
    debug_assert!(people.iter().all(|person| person.id != id));

    people_save_all(&people)?;

    groups_member_drop(id)
}

pub(crate) fn groups_load_all() -> AppResult<Vec<Group>> {
    let path = workspace::root_file_path(GROUPS_FILE)?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = workspace::file_read_bounded(&path, GROUPS_FILE_BYTES_MAX)?;

    let groups: Vec<Group> = serde_json::from_str(&raw).map_err(|error| {
        AppError::Config(format!("the saved groups could not be read: {error}"))
    })?;

    groups_count_validate(&groups)?;

    Ok(groups)
}

fn groups_count_validate(groups: &[Group]) -> AppResult<()> {
    if groups.len() > GROUP_COUNT_MAX as usize {
        return Err(AppError::Config(format!(
            "there are {} groups, over the {GROUP_COUNT_MAX} this app keeps",
            groups.len()
        )));
    }

    Ok(())
}

fn groups_save_all(groups: &[Group]) -> AppResult<()> {
    groups_count_validate(groups)?;

    let path = workspace::root_file_path(GROUPS_FILE)?;
    let encoded = serde_json::to_string_pretty(groups)?;

    debug_assert!(encoded.len() <= GROUPS_FILE_BYTES_MAX as usize);

    workspace::atomic_write(&path, encoded.as_bytes())
}

pub(crate) fn group_upsert(group: &Group) -> AppResult<()> {
    let group = group_normalize(group);

    group_validate(&group)?;

    let id = group.id.clone();
    let mut groups = groups_load_all()?;

    match groups.iter_mut().find(|existing| existing.id == group.id) {
        Some(existing) => *existing = group,
        None => groups.push(group),
    }

    debug_assert_eq!(groups.iter().filter(|existing| existing.id == id).count(), 1);

    groups_save_all(&groups)
}

pub(crate) fn group_delete(id: &str) -> AppResult<()> {
    let mut groups = groups_load_all()?;
    let groups_count_before = groups.len();

    groups.retain(|group| group.id != id);

    debug_assert!(groups.len() <= groups_count_before);
    debug_assert!(groups.iter().all(|group| group.id != id));

    groups_save_all(&groups)
}

fn groups_member_drop(person_id: &str) -> AppResult<()> {
    let mut groups = groups_load_all()?;

    for group in &mut groups {
        group.person_ids.retain(|id| id != person_id);
    }

    debug_assert!(groups.iter().all(|group| !group.person_ids.iter().any(|id| id == person_id)));

    groups_save_all(&groups)
}

fn group_normalize(group: &Group) -> Group {
    let mut person_ids: Vec<String> = Vec::with_capacity(group.person_ids.len());

    for id in &group.person_ids {
        let trimmed = id.trim();

        if trimmed.is_empty() {
            continue;
        }

        if person_ids.iter().any(|kept| kept == trimmed) {
            continue;
        }

        person_ids.push(trimmed.to_owned());
    }

    debug_assert!(person_ids.len() <= group.person_ids.len());
    debug_assert!(person_ids.iter().all(|id| !id.is_empty()));

    Group {
        id: group.id.trim().to_owned(),
        name: group.name.trim().to_owned(),
        person_ids,
    }
}

fn group_validate(group: &Group) -> AppResult<()> {
    if group.id.is_empty() {
        return Err(AppError::Config("the group has no id".into()));
    }

    if group.name.is_empty() {
        return Err(AppError::Config("give the group a name before saving".into()));
    }

    if group.name.chars().count() > GROUP_NAME_CHARS_MAX as usize {
        return Err(AppError::Config(format!(
            "the group name is too long; keep it under {GROUP_NAME_CHARS_MAX} characters"
        )));
    }

    if group.person_ids.len() > PEOPLE_COUNT_MAX as usize {
        return Err(AppError::Config(format!(
            "the group has {} members, over the {PEOPLE_COUNT_MAX} people this app keeps",
            group.person_ids.len()
        )));
    }

    Ok(())
}

fn person_normalize(person: &Person) -> Person {
    let normalized = Person {
        id: person.id.trim().to_owned(),
        name_first: person.name_first.trim().to_owned(),
        name_last: person.name_last.trim().to_owned(),
        role: person.role.trim().to_owned(),
        description: person.description.trim().to_owned(),
    };

    debug_assert!(normalized.name_first.len() <= person.name_first.len());
    debug_assert!(normalized.description.len() <= person.description.len());

    normalized
}

fn person_validate(person: &Person) -> AppResult<()> {
    if person.id.is_empty() {
        return Err(AppError::Config("the person has no id".into()));
    }

    if person.name_first.is_empty() {
        return Err(AppError::Config("give the person a first name before saving".into()));
    }

    if person.name_first.chars().count() > PERSON_NAME_CHARS_MAX as usize {
        return Err(AppError::Config(format!(
            "the first name is too long; keep it under {PERSON_NAME_CHARS_MAX} characters"
        )));
    }

    if person.name_last.chars().count() > PERSON_NAME_CHARS_MAX as usize {
        return Err(AppError::Config(format!(
            "the last name is too long; keep it under {PERSON_NAME_CHARS_MAX} characters"
        )));
    }

    if person.role.chars().count() > PERSON_ROLE_CHARS_MAX as usize {
        return Err(AppError::Config(format!(
            "the role is too long; keep it under {PERSON_ROLE_CHARS_MAX} characters"
        )));
    }

    if person.description.chars().count() > PERSON_DESCRIPTION_CHARS_MAX as usize {
        return Err(AppError::Config(format!(
            "the description is too long; keep it under {PERSON_DESCRIPTION_CHARS_MAX} \
             characters"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::test_support::root_scoped;
    use std::fs;

    fn person_with(name_first: &str, name_last: &str) -> Person {
        Person {
            id: "test".to_owned(),
            name_first: name_first.to_owned(),
            name_last: name_last.to_owned(),
            role: String::new(),
            description: String::new(),
        }
    }

    #[test]
    fn a_person_saved_under_the_old_name_keys_still_loads() {
        let raw = r#"{"id":"1","first_name":"John","last_name":"Doe"}"#;
        let person: Person = serde_json::from_str(raw).unwrap();

        assert_eq!(person.name_full(), "John Doe");
    }

    #[test]
    fn a_person_without_a_first_name_is_refused() {
        assert!(person_validate(&person_normalize(&person_with("  ", "Doe"))).is_err());
        assert!(person_validate(&person_normalize(&person_with("John", ""))).is_ok());
    }

    #[test]
    fn a_person_over_a_size_limit_is_refused_and_length_is_counted_in_characters() {
        let name_long = "n".repeat(PERSON_NAME_CHARS_MAX as usize + 1);
        let name_multibyte = "é".repeat(PERSON_NAME_CHARS_MAX as usize);

        assert!(person_validate(&person_with(&name_long, "Doe")).is_err());
        assert!(person_validate(&person_with("John", &name_long)).is_err());
        assert!(person_validate(&person_with(&name_multibyte, "Doe")).is_ok());
    }

    #[test]
    fn the_full_name_drops_a_missing_last_name() {
        assert_eq!(person_with("John", "").name_full(), "John");
        assert_eq!(person_with("John", "Doe").name_full(), "John Doe");
    }

    #[test]
    fn a_group_keeps_each_member_once_and_needs_a_name() {
        let group = group_normalize(&Group {
            id: " g ".to_owned(),
            name: " Managers ".to_owned(),
            person_ids: vec!["a".to_owned(), " a ".to_owned(), String::new(), "b".to_owned()],
        });

        assert_eq!(group.person_ids, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(group.name, "Managers");
        assert!(group_validate(&group).is_ok());
        assert!(group_validate(&Group { name: String::new(), ..group }).is_err());
    }

    #[test]
    fn a_count_past_its_limit_is_refused() {
        let person = person_with("John", "Doe");
        let group = Group { id: "g".to_owned(), name: "Managers".to_owned(), person_ids: vec![] };

        let people_at_limit = vec![person.clone(); PEOPLE_COUNT_MAX as usize];
        let people_past_limit = vec![person; PEOPLE_COUNT_MAX as usize + 1];
        let groups_at_limit = vec![group.clone(); GROUP_COUNT_MAX as usize];
        let groups_past_limit = vec![group; GROUP_COUNT_MAX as usize + 1];

        assert!(people_count_validate(&people_at_limit).is_ok());
        assert!(people_count_validate(&people_past_limit).is_err());
        assert!(groups_count_validate(&groups_at_limit).is_ok());
        assert!(groups_count_validate(&groups_past_limit).is_err());
    }

    #[test]
    fn a_person_is_trimmed_on_every_field_and_capped_on_role_and_description() {
        let mut person = person_with(" Jane ", " Doe ");
        person.id = " id ".to_owned();
        person.role = " PM ".to_owned();
        person.description = " runs delivery ".to_owned();
        let normalized = person_normalize(&person);
        let mut roled = person_with("Jane", "Doe");
        roled.role = "r".repeat(PERSON_ROLE_CHARS_MAX as usize + 1);
        let mut described = person_with("Jane", "Doe");
        described.description = "d".repeat(PERSON_DESCRIPTION_CHARS_MAX as usize + 1);
        let mut unidentified = person_with("Jane", "Doe");
        unidentified.id = String::new();

        assert_eq!(normalized.id, "id");
        assert_eq!(normalized.name_full(), "Jane Doe");
        assert_eq!(normalized.role, "PM");
        assert_eq!(normalized.description, "runs delivery");
        assert!(person_validate(&roled).is_err());
        assert!(person_validate(&described).is_err());
        assert!(person_validate(&unidentified).is_err());
    }

    #[test]
    fn a_group_is_capped_on_name_and_membership_and_needs_an_id() {
        let name_at_limit = "n".repeat(GROUP_NAME_CHARS_MAX as usize);
        let group = Group { id: "g".to_owned(), name: name_at_limit, person_ids: vec![] };
        let name_past_limit = "n".repeat(GROUP_NAME_CHARS_MAX as usize + 1);
        let overnamed = Group { name: name_past_limit, ..group.clone() };
        let members = vec!["p".to_owned(); PEOPLE_COUNT_MAX as usize + 1];
        let crowded = Group { person_ids: members, ..group.clone() };
        let unidentified = Group { id: String::new(), ..group.clone() };
        let restored: Group = serde_json::from_str(r#"{"id":"g","name":"Team"}"#).unwrap();

        assert!(group_validate(&group).is_ok());
        assert!(group_validate(&overnamed).is_err());
        assert!(group_validate(&crowded).is_err());
        assert!(group_validate(&unidentified).is_err());
        assert_eq!(restored.person_ids.len(), 0);
    }

    #[test]
    fn people_are_saved_once_per_id_and_load_back_normalized() {
        let _root = root_scoped("people-store");
        let mut person = person_with(" Jane ", " Doe ");
        person.role = " PM ".to_owned();

        assert_eq!(people_load_all().unwrap().len(), 0);

        person_upsert(&person).unwrap();
        person.name_last = "Roe".to_owned();
        person_upsert(&person).unwrap();

        let people = people_load_all().unwrap();

        assert_eq!(people.len(), 1);
        assert_eq!(people[0].name_full(), "Jane Roe");
        assert_eq!(people[0].role, "PM");
        assert!(person_upsert(&person_with("", "Doe")).is_err());
        assert_eq!(people_load_all().unwrap().len(), 1);
    }

    #[test]
    fn deleting_a_person_also_drops_them_from_every_group() {
        let _root = root_scoped("people-delete");

        let group = Group {
            id: "g".to_owned(),
            name: "Team".to_owned(),
            person_ids: vec!["test".to_owned(), "other".to_owned()],
        };

        person_upsert(&person_with("Jane", "Doe")).unwrap();
        group_upsert(&group).unwrap();
        person_delete("test").unwrap();

        assert_eq!(people_load_all().unwrap().len(), 0);
        assert_eq!(groups_load_all().unwrap()[0].person_ids, vec!["other".to_owned()]);

        person_delete("test").unwrap();

        assert_eq!(groups_load_all().unwrap().len(), 1);
    }

    #[test]
    fn groups_are_saved_once_per_id_and_removed_by_id() {
        let _root = root_scoped("groups-store");
        let group = Group { id: " g ".to_owned(), name: " Team ".to_owned(), person_ids: vec![] };

        assert_eq!(groups_load_all().unwrap().len(), 0);

        group_upsert(&group).unwrap();
        group_upsert(&Group { name: "Renamed".to_owned(), ..group.clone() }).unwrap();

        let groups = groups_load_all().unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "g");
        assert_eq!(groups[0].name, "Renamed");

        group_delete("g").unwrap();

        assert_eq!(groups_load_all().unwrap().len(), 0);
        assert!(group_upsert(&Group { id: String::new(), ..group }).is_err());
    }

    #[test]
    fn a_damaged_people_or_groups_file_is_an_error_rather_than_an_empty_list() {
        let _root = root_scoped("people-damaged");

        fs::write(workspace::root_file_path(PEOPLE_FILE).unwrap(), b"[{").unwrap();
        fs::write(workspace::root_file_path(GROUPS_FILE).unwrap(), b"nope").unwrap();

        assert!(people_load_all().is_err());
        assert!(groups_load_all().is_err());
        assert!(person_upsert(&person_with("Jane", "Doe")).is_err());
        assert!(group_delete("g").is_err());
    }
}
