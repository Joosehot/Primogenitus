//! The dictionary: WordNet 3.1, downloaded once and read as it is.
//!
//! data.*  every sense: its words, its lines to other senses, a verb's sentence
//!         frames, the gloss with its example sentences
//! index.* every word: its senses, most common first, and how often it was
//!         tagged in use (familiarity)
//! *.exc   irregular forms (ate -> eat, mice -> mouse, better -> good)

use crate::network::Pos;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Synset {
    pub lexfile: u8,
    /// (word as written, its adjective marker such as "(a)" or "")
    pub words: Vec<(String, String)>,
    pub ptrs: Vec<Ptr>,
    /// (frame number, word number; 0 = every word of the sense)
    pub frames: Vec<(u8, u8)>,
    pub gloss: String,
}

pub struct Ptr {
    pub sym: String,
    pub target: (Pos, u32),
    pub src: u8,
    pub tgt: u8,
}

pub struct IndexEntry {
    pub tagsense: u32,
    pub offsets: Vec<u32>,
}

pub struct WordNet {
    pub synsets: BTreeMap<(Pos, u32), Synset>,
    pub index: BTreeMap<(Pos, String), IndexEntry>,
    /// base -> its irregular forms
    pub exc: BTreeMap<(Pos, String), Vec<String>>,
}

pub fn pos_of(c: &str) -> Option<Pos> {
    match c {
        "n" => Some(Pos::Noun),
        "v" => Some(Pos::Verb),
        "a" | "s" => Some(Pos::Adj),
        "r" => Some(Pos::Adv),
        _ => None,
    }
}

const FILES: [(&str, Pos); 4] = [("noun", Pos::Noun), ("verb", Pos::Verb), ("adj", Pos::Adj), ("adv", Pos::Adv)];

/// The dictionary's folder, downloaded and unpacked the first time.
pub fn fetch(root: &Path, url: &str, log: &mut dyn FnMut(String)) -> Result<PathBuf> {
    let dir = root.join("data").join("wordnet");
    if let Some(d) = find_dict(&dir) {
        return Ok(d);
    }
    fs::create_dir_all(&dir)?;
    let tgz = dir.join("wordnet.tar.gz");
    if !tgz.exists() {
        log(format!("downloading {url}"));
        let resp = ureq::get(url).call().with_context(|| format!("downloading {url}"))?;
        let tmp = dir.join("wordnet.tar.gz.part");
        let mut f = fs::File::create(&tmp)?;
        std::io::copy(&mut resp.into_reader(), &mut f)?;
        drop(f);
        fs::rename(&tmp, &tgz)?;
    }
    let gz = flate2::read::GzDecoder::new(fs::File::open(&tgz)?);
    tar::Archive::new(gz).unpack(&dir).context("unpacking the dictionary")?;
    match find_dict(&dir) {
        Some(d) => Ok(d),
        None => bail!("no data.noun inside {}", dir.display()),
    }
}

fn find_dict(dir: &Path) -> Option<PathBuf> {
    for cand in [dir.join("dict"), dir.to_path_buf()] {
        if cand.join("data.noun").exists() {
            return Some(cand);
        }
    }
    None
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn read(dir: &Path) -> Result<WordNet> {
    let mut synsets = BTreeMap::new();
    let mut index = BTreeMap::new();
    let mut exc: BTreeMap<(Pos, String), Vec<String>> = BTreeMap::new();
    for (name, pos) in FILES {
        let text = read_text(&dir.join(format!("data.{name}")))?;
        for line in text.lines() {
            if line.starts_with("  ") || line.trim().is_empty() {
                continue;
            }
            let (head, gloss) = line.split_once("| ").unwrap_or((line, ""));
            let f: Vec<&str> = head.split_whitespace().collect();
            let offset: u32 = f[0].parse()?;
            let lexfile: u8 = f[1].parse()?;
            let w_cnt = usize::from_str_radix(f[3], 16)?;
            let mut p = 4;
            let mut words = Vec::with_capacity(w_cnt);
            for _ in 0..w_cnt {
                let w = f[p];
                p += 2;
                match w.find('(') {
                    Some(k) => words.push((w[..k].to_string(), w[k..].to_string())),
                    None => words.push((w.to_string(), String::new())),
                }
            }
            let p_cnt: usize = f[p].parse()?;
            p += 1;
            let mut ptrs = Vec::with_capacity(p_cnt);
            for _ in 0..p_cnt {
                let st = f[p + 3];
                if let Some(tp) = pos_of(f[p + 2]) {
                    ptrs.push(Ptr {
                        sym: f[p].to_string(),
                        target: (tp, f[p + 1].parse()?),
                        src: u8::from_str_radix(&st[..2], 16)?,
                        tgt: u8::from_str_radix(&st[2..4], 16)?,
                    });
                }
                p += 4;
            }
            let mut frames = Vec::new();
            if pos == Pos::Verb && p < f.len() {
                let f_cnt: usize = f[p].parse()?;
                p += 1;
                for _ in 0..f_cnt {
                    frames.push((f[p + 1].parse()?, u8::from_str_radix(f[p + 2], 16)?));
                    p += 3;
                }
            }
            synsets.insert((pos, offset), Synset { lexfile, words, ptrs, frames, gloss: gloss.trim().to_string() });
        }
        let text = read_text(&dir.join(format!("index.{name}")))?;
        for line in text.lines() {
            if line.starts_with("  ") || line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split_whitespace().collect();
            let synset_cnt: usize = f[2].parse()?;
            let p_cnt: usize = f[3].parse()?;
            let at = 4 + p_cnt;
            let tagsense: u32 = f[at + 1].parse()?;
            let offsets = f[at + 2..at + 2 + synset_cnt].iter().map(|s| s.parse()).collect::<Result<Vec<u32>, _>>()?;
            index.insert((pos, f[0].to_string()), IndexEntry { tagsense, offsets });
        }
        let path = dir.join(format!("{name}.exc"));
        if path.exists() {
            for line in read_text(&path)?.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() < 2 {
                    continue;
                }
                for base in &f[1..] {
                    exc.entry((pos, base.to_string())).or_default().push(f[0].to_string());
                }
            }
        }
    }
    Ok(WordNet { synsets, index, exc })
}
