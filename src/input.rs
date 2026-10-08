//! What starts a sentence. Not decided yet, so the simplest option: one word,
//! a noun (the topic), and the sentence is about it.
//!
//! To swap: add a variant here (a sentence to answer, a word and a verb, ...)
//! and say in `start` how it becomes the node the meaning spreads from.

use crate::network::Network;
use crate::parser;
use anyhow::Result;

pub enum Input {
    /// One noun: "dog".
    Topic(String),
}

impl Input {
    pub fn from_args(words: &[String]) -> Input {
        Input::Topic(words.join(" "))
    }

    /// The node the meaning spreads from.
    pub fn start(&self, net: &Network) -> Result<u32> {
        match self {
            Input::Topic(w) => parser::read_topic(net, w),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Input::Topic(w) => format!("the topic \"{w}\" (one noun; the sentence is about it)"),
        }
    }
}
