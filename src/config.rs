//! rules.toml: every number Primogenitus uses. Unknown or missing keys fail at load.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub build: Build,
    pub lines: Lines,
    pub spread: Spread,
    pub search: Search,
    pub score: Score,
    pub patterns: Patterns,
    pub article: Article,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub url: String,
    pub semantic_targets: usize,
    pub frame_senses: usize,
    pub mass_lexfiles: Vec<u8>,
    pub count_evidence: usize,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Lines {
    pub form_of: f32,
    pub synonym: f32,
    pub hypernym: f32,
    pub hyponym: f32,
    pub has_part: f32,
    pub part_of: f32,
    pub has_member: f32,
    pub member_of: f32,
    pub made_of: f32,
    pub substance_of: f32,
    pub attribute: f32,
    pub derived: f32,
    pub antonym: f32,
    pub similar: f32,
    pub entails: f32,
    pub causes: f32,
    pub pertains: f32,
    pub subject_of: f32,
    pub has_subject: f32,
    pub object_of: f32,
    pub has_object: f32,
    pub modifies: f32,
    pub modified_by: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Spread {
    pub hops: usize,
    pub keep: usize,
    pub rank_decay: f32,
    pub degree_power: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub beam: usize,
    pub seeds: usize,
    pub candidates: usize,
    pub jitter: f64,
    pub subject_adjectives: usize,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Score {
    pub meaning: f64,
    pub familiarity: f64,
    pub rare: f64,
    pub object_line: f64,
    pub animacy: f64,
    pub past: f64,
    pub generic_a: f64,
    pub adjective: f64,
    pub hop2: f64,
    pub role_subject: f64,
    pub role_object: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Patterns {
    pub is_a: f64,
    pub has_part: f64,
    pub does: f64,
    pub does_to: f64,
    pub is_adj: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Article {
    pub an: Vec<String>,
    pub a: Vec<String>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn line_weight(&self, kind: crate::network::Kind) -> f32 {
        use crate::network::Kind::*;
        let l = &self.lines;
        match kind {
            FormOf => l.form_of,
            Synonym => l.synonym,
            Hypernym => l.hypernym,
            Hyponym => l.hyponym,
            HasPart => l.has_part,
            PartOf => l.part_of,
            HasMember => l.has_member,
            MemberOf => l.member_of,
            MadeOf => l.made_of,
            SubstanceOf => l.substance_of,
            Attribute => l.attribute,
            Derived => l.derived,
            Antonym => l.antonym,
            Similar => l.similar,
            Entails => l.entails,
            Causes => l.causes,
            Pertains => l.pertains,
            SubjectOf => l.subject_of,
            HasSubject => l.has_subject,
            ObjectOf => l.object_of,
            HasObject => l.has_object,
            Modifies => l.modifies,
            ModifiedBy => l.modified_by,
        }
    }
}
