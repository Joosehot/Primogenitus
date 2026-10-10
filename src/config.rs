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
    pub chat: Chat,
    pub me: Me,
    pub raven: RavenCfg,
    pub mblm: MblmCfg,
}

/// MBLM, the motor based language model: its beam, its softmax temperature,
/// and the priors of the grammar's choices.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct MblmCfg {
    pub sentences: usize,
    pub candidates: usize,
    pub beam: usize,
    pub temperature: f64,
    pub said: f64,
    pub pronoun: f64,
    pub noun_again: f64,
    pub a: f64,
    pub the: f64,
    pub bare: f64,
    pub adjective: f64,
    pub is: f64,
    pub has: f64,
    pub does: f64,
    pub end_intransitive: f64,
    pub end_with_objects: f64,
}

/// Raven: the words it says around Primo's facts, its judges' weights, its
/// feeling anchors.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RavenCfg {
    pub say: std::collections::BTreeMap<String, Vec<String>>,
    pub judges: RavenJudges,
    pub feeling: Feeling,
    pub memory: RavenMemory,
}

/// How much of the conversation is kept as it was said; past it, the oldest
/// exchanges are compacted (their words let go, their facts kept).
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RavenMemory {
    pub max_words: usize,
    pub keep_words: usize,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RavenJudges {
    pub feelings_first: f64,
    pub facts_in_pain: f64,
    pub shares_joy: f64,
    pub answers: f64,
    pub listens: f64,
    pub unasked_fact: f64,
    pub remembers: f64,
    pub name_use: f64,
    pub name_overuse: f64,
    pub keeps_going: f64,
    pub interrogation: f64,
    pub repeat: f64,
    pub generic: f64,
    pub max_words: usize,
    pub low_turns: u64,
    pub off_thread: f64,
    pub stays: f64,
    pub too_long: f64,
    pub jitter: f64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Feeling {
    pub good_adjectives: Vec<String>,
    pub mild_adjectives: Vec<String>,
    pub bad_adjectives: Vec<String>,
    pub good_nouns: Vec<String>,
    pub bad_nouns: Vec<String>,
    pub good_events: Vec<String>,
    pub bad_events: Vec<String>,
}

/// What Primo knows about itself.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Me {
    pub kinds: Vec<String>,
    pub not_kind: String,
    pub cannot: String,
    pub unknown_adj: String,
    pub can: std::collections::BTreeMap<String, String>,
    pub is: std::collections::BTreeMap<String, String>,
    pub has: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Chat {
    pub greeting: String,
    pub goodbye: String,
    pub thanks: String,
    pub help: String,
    pub no_pronoun: String,
    pub nothing_yet: String,
    pub who_am_i: String,
    pub how_am_i: String,
    pub cannot: String,
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
