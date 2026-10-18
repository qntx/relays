//! Parsing and validation of the `relays.toml` curation file.
//!
//! Validation runs in two stages: serde deserialisation reports structural
//! errors (unknown fields, wrong types, missing fields) with line and
//! column, stopping at the first error; a semantic pass then collects every
//! remaining violation.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::url::RelayUrl;

const MAX_NAME_CHARS: usize = 40;

/// Text in multiple languages, keyed by BCP 47 language tag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Localized(BTreeMap<String, String>);

impl Localized {
    /// Wrap an existing language map.
    #[must_use]
    pub const fn new(map: BTreeMap<String, String>) -> Self {
        Self(map)
    }

    /// Text for a language tag.
    #[must_use]
    pub fn get(&self, lang: &str) -> Option<&str> {
        self.0.get(lang).map(String::as_str)
    }

    /// Iterate over `(tag, text)` pairs in tag order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A curated collection with its ordered member list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    /// Kebab-case identifier, unique within the file.
    pub id: String,
    /// Display name; must contain `en`.
    pub name: Localized,
    /// Optional description; must contain `en` when present.
    pub description: Option<Localized>,
    /// Member URLs in editorial order.
    pub relays: Vec<RelayUrl>,
}

/// Per-relay editorial overrides. Present only when at least one field is
/// needed; `languages` may be empty and `nsfw` defaults to `false`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayOverride {
    /// Display name, at most 40 characters.
    pub name: Option<String>,
    /// Localized description; must contain `en` when present.
    pub description: Option<Localized>,
    /// ISO 639-1 language codes declared by the relay.
    pub languages: Vec<String>,
    /// Not-safe-for-work flag.
    pub nsfw: bool,
}

/// A validated curation file.
#[derive(Debug, Clone, Default)]
pub struct Curation {
    /// Collections in display order.
    pub collections: Vec<Collection>,
    /// Relay overrides keyed by URL; every key is referenced by a collection.
    pub relays: BTreeMap<RelayUrl, RelayOverride>,
}

/// All validation errors found in a curation file.
#[derive(Debug)]
pub struct CurationErrors(pub Vec<String>);

impl fmt::Display for CurationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            f.write_str(error)?;
        }
        Ok(())
    }
}

impl Error for CurationErrors {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    collection: Vec<RawCollection>,
    #[serde(default)]
    relay: Vec<RawRelay>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCollection {
    id: String,
    name: Localized,
    description: Option<Localized>,
    relays: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRelay {
    url: String,
    name: Option<String>,
    description: Option<Localized>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    nsfw: bool,
}

impl Curation {
    /// Parse and validate a curation file.
    ///
    /// # Errors
    ///
    /// Returns [`CurationErrors`]: a single structural error with line and
    /// column when the document does not match the schema, or every semantic
    /// violation found.
    pub fn parse(source: &str) -> Result<Self, CurationErrors> {
        let raw: File = match toml::from_str(source) {
            Ok(raw) => raw,
            Err(e) => return Err(CurationErrors(vec![format!("{e}")])),
        };

        let mut errors = Vec::new();
        let mut ids = BTreeSet::new();
        let mut collections = Vec::with_capacity(raw.collection.len());
        for collection in raw.collection {
            if !ids.insert(collection.id.clone()) {
                errors.push(format!("collection `{}` is defined twice", collection.id));
            }
            collections.push(collection_entry(collection, &mut errors));
        }

        let member_urls: BTreeSet<&RelayUrl> =
            collections.iter().flat_map(|c| c.relays.iter()).collect();
        let mut relays = BTreeMap::new();
        for relay in raw.relay {
            let ctx = format!("relay `{}`", relay.url);
            let url = match canonical_url(&relay.url) {
                Ok(url) => url,
                Err(e) => {
                    errors.push(format!("{ctx}: {e}"));
                    continue;
                }
            };
            if !member_urls.contains(&url) {
                errors.push(format!(
                    "`[[relay]]` for `{url}` is not referenced by any collection"
                ));
            }
            let entry = relay_entry(&ctx, &relay, &mut errors);
            if relays.insert(url.clone(), entry).is_some() {
                errors.push(format!("`[[relay]]` for `{url}` is defined twice"));
            }
        }

        if errors.is_empty() {
            Ok(Self {
                collections,
                relays,
            })
        } else {
            Err(CurationErrors(errors))
        }
    }

    /// All URLs referenced by at least one collection, sorted.
    #[must_use]
    pub fn urls(&self) -> BTreeSet<&RelayUrl> {
        self.collections
            .iter()
            .flat_map(|c| c.relays.iter())
            .collect()
    }
}

/// Validate a localized table: BCP 47 tags, non-empty values, the `en`
/// requirement and the reserved `und` tag.
fn check_localized(ctx: &str, key: &str, localized: &Localized, errors: &mut Vec<String>) {
    for (tag, text) in localized.iter() {
        if text.is_empty() {
            errors.push(format!("{ctx}: `{key}.{tag}` must be a non-empty string"));
        }
        if tag == "und" {
            errors.push(format!("{ctx}: `{key}` uses the reserved tag `und`"));
        } else if !valid_tag(tag) {
            errors.push(format!("{ctx}: `{tag}` is not a BCP 47 language tag"));
        }
    }
    if localized.get("en").is_none() {
        errors.push(format!("{ctx}: `{key}` must contain `en`"));
    }
}

fn canonical_url(raw: &str) -> Result<RelayUrl, String> {
    let url = RelayUrl::parse(raw).map_err(|_| format!("`{raw}` is not a valid relay URL"))?;
    if !url.is_secure() {
        return Err(format!("`{raw}` is not a `wss://` URL"));
    }
    if !url.inner().username().is_empty() || url.inner().password().is_some() {
        return Err(format!("`{raw}` must not contain userinfo"));
    }
    if url.as_str() != raw {
        return Err(format!("`{raw}` is not canonical; use `{url}`"));
    }
    Ok(url)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.split('-').all(|part| {
            (1..=8).contains(&part.len()) && part.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

fn collection_entry(raw: RawCollection, errors: &mut Vec<String>) -> Collection {
    let ctx = format!("collection `{}`", raw.id);
    if !valid_id(&raw.id) {
        errors.push(format!("{ctx}: id `{}` is not kebab-case", raw.id));
    } else if raw.id.starts_with("role-") || raw.id.starts_with("lang-") {
        errors.push(format!(
            "{ctx}: id `{}` is reserved for automatic collections",
            raw.id
        ));
    }
    check_localized(&ctx, "name", &raw.name, errors);
    if let Some(description) = &raw.description {
        check_localized(&ctx, "description", description, errors);
    }
    if raw.relays.is_empty() {
        errors.push(format!("{ctx}: `relays` must not be empty"));
    }
    let mut seen = BTreeSet::new();
    let mut relays = Vec::with_capacity(raw.relays.len());
    for raw_url in &raw.relays {
        match canonical_url(raw_url) {
            Ok(url) if seen.insert(url.clone()) => relays.push(url),
            Ok(_) => errors.push(format!("{ctx}: `{raw_url}` is listed twice")),
            Err(e) => errors.push(format!("{ctx}: {e}")),
        }
    }
    Collection {
        id: raw.id,
        name: raw.name,
        description: raw.description,
        relays,
    }
}

fn relay_entry(ctx: &str, raw: &RawRelay, errors: &mut Vec<String>) -> RelayOverride {
    if let Some(name) = &raw.name {
        let chars = name.chars().count();
        if chars == 0 || chars > MAX_NAME_CHARS {
            errors.push(format!(
                "{ctx}: `name` must be 1 to {MAX_NAME_CHARS} characters"
            ));
        }
    }
    if let Some(description) = &raw.description {
        check_localized(ctx, "description", description, errors);
    }
    let mut languages = Vec::with_capacity(raw.languages.len());
    for code in &raw.languages {
        if !iso639_1(code) {
            errors.push(format!(
                "{ctx}: `{code}` is not an ISO 639-1 two-letter code"
            ));
        } else if languages.contains(code) {
            errors.push(format!("{ctx}: language `{code}` is listed twice"));
        }
        languages.push(code.clone());
    }
    RelayOverride {
        name: raw.name.clone(),
        description: raw.description.clone(),
        languages,
        nsfw: raw.nsfw,
    }
}

fn iso639_1(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase())
}
