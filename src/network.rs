//! The frozen word network: every word and every variation of a word is a
//! node; lines run from word to word and carry the meaning (is a kind of, has
//! as a part, does, is done to, describes, ...). Built once from the
//! dictionary by `build`, frozen to data/network.bin, never updated after.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Pos {
    Noun,
    Verb,
    Adj,
    Adv,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Form {
    Base,
    Plural,
    ThirdSg,
    Past,
    PastPart,
    Ing,
    Comparative,
    Superlative,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Kind {
    FormOf,
    Synonym,
    Hypernym,
    Hyponym,
    HasPart,
    PartOf,
    HasMember,
    MemberOf,
    MadeOf,
    SubstanceOf,
    Attribute,
    Derived,
    Antonym,
    Similar,
    Entails,
    Causes,
    Pertains,
    SubjectOf,
    HasSubject,
    ObjectOf,
    HasObject,
    Modifies,
    ModifiedBy,
    /// read from the definitions: what it eats ("feeds on fish", "flesh-eating")
    Eats,
    EatenBy,
    /// where it lives ("living in water", "found in Asia", "aquatic")
    LivesIn,
    HomeOf,
    /// what it is used for ("used for pounding")
    UsedFor,
    UseOf,
}

impl Kind {
    pub fn name(self) -> &'static str {
        use Kind::*;
        match self {
            FormOf => "form of",
            Synonym => "same as",
            Hypernym => "is a",
            Hyponym => "has kind",
            HasPart => "has part",
            PartOf => "part of",
            HasMember => "has member",
            MemberOf => "member of",
            MadeOf => "made of",
            SubstanceOf => "substance of",
            Attribute => "attribute",
            Derived => "derived",
            Antonym => "opposite",
            Similar => "similar",
            Entails => "entails",
            Causes => "causes",
            Pertains => "pertains",
            SubjectOf => "does",
            HasSubject => "done by",
            ObjectOf => "undergoes",
            HasObject => "done to",
            Modifies => "describes",
            ModifiedBy => "described as",
            Eats => "eats",
            EatenBy => "eaten by",
            LivesIn => "lives in",
            HomeOf => "home of",
            UsedFor => "used for",
            UseOf => "use of",
        }
    }
}

/// Adjective markers from the dictionary.
pub const ATTRIBUTIVE_ONLY: u8 = 1; // (a): "former", only before a noun
pub const PREDICATIVE_ONLY: u8 = 2; // (p): "afraid", only after "is"
pub const POSTNOMINAL: u8 = 3; // (ip): "galore", only right after a noun

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Node {
    pub text: String,
    pub pos: Pos,
    pub form: Form,
    /// the base word's node (itself for a base word)
    pub lemma: u32,
    /// how often the word was tagged in use (WordNet's tag counts)
    pub fam: u16,
    pub proper: bool,
    /// a verb's sentence frames (bit n = WordNet frame n)
    pub frames: u64,
    /// a noun whose most common sense is a person or an animal
    pub animate: bool,
    pub marker: u8,
    /// a noun that may be uncountable: only "the X" (singular) is said
    pub mass: bool,
    /// the form is known to be right (unsure forms are never generated)
    pub sure: bool,
    /// the first word of its most common sense (the sense's main name: "Helsinki", not "Helsingfors")
    #[serde(default)]
    pub head: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Line {
    pub to: u32,
    pub kind: Kind,
    /// which sense of the word the line comes from (0 = the most common)
    pub rank: u8,
    /// which sense of the target word it lands in (UNKNOWN when not known):
    /// a walk of two steps goes on only from the sense it landed in
    pub to_rank: u8,
}

pub const UNKNOWN: u8 = 255;

#[derive(Serialize, Deserialize, Default)]
pub struct Network {
    pub nodes: Vec<Node>,
    pub lines: Vec<Vec<Line>>,
    /// base node -> its variation nodes
    pub forms: Vec<Vec<u32>>,
    /// a base word's definition in its most common sense, as the dictionary has it
    #[serde(default)]
    pub defs: HashMap<u32, String>,
    #[serde(skip)]
    index: HashMap<String, Vec<u32>>,
}

impl Network {
    pub fn new(nodes: Vec<Node>, lines: Vec<Vec<Line>>, forms: Vec<Vec<u32>>) -> Network {
        let mut n = Network { nodes, lines, forms, defs: HashMap::new(), index: HashMap::new() };
        n.reindex();
        n
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (i, n) in self.nodes.iter().enumerate() {
            self.index.entry(n.text.to_lowercase()).or_default().push(i as u32);
        }
    }

    /// Every node written this way.
    pub fn lookup(&self, text: &str) -> &[u32] {
        self.index.get(&text.to_lowercase()).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The base word's node of this part of speech.
    pub fn base(&self, text: &str, pos: Pos) -> Option<u32> {
        self.lookup(text).iter().copied().find(|&i| {
            let n = &self.nodes[i as usize];
            n.pos == pos && n.form == Form::Base
        })
    }

    /// A base word's variation (the base itself for Form::Base).
    pub fn form(&self, lemma: u32, form: Form) -> Option<&Node> {
        if form == Form::Base {
            return Some(&self.nodes[lemma as usize]);
        }
        self.forms[lemma as usize].iter().map(|&i| &self.nodes[i as usize]).find(|n| n.form == form)
    }

    /// The dictionary's definition of a base word (its most common sense).
    pub fn def(&self, i: u32) -> Option<&str> {
        self.defs.get(&i).map(|s| s.as_str())
    }

    pub fn node(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(|s| s.as_str())
    }

    pub fn line_count(&self) -> usize {
        self.lines.iter().map(|l| l.len()).sum()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let f = std::io::BufWriter::new(std::fs::File::create(path)?);
        bincode::serialize_into(f, self).context("freezing the network")?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Network> {
        let f = std::io::BufReader::new(
            std::fs::File::open(path).with_context(|| format!("{} not found: run `primogenitus build` first", path.display()))?,
        );
        let mut n: Network = bincode::deserialize_from(f).context("reading the frozen network")?;
        n.reindex();
        Ok(n)
    }
}
