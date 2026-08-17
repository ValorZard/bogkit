use std::collections::{BTreeMap, HashMap, HashSet};

use iddqd::{BiHashItem, BiHashMap, bi_upcast};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A tally of named affinity scores — `{"romance": 1}` — used both for what a
/// node *awards* and what a node *demands*.
///
/// `BTreeMap` rather than `HashMap` because [`DialogueNodeData`] is the second
/// key of the dialogue map and so must be `Hash + Eq`, which `HashMap` isn't.
pub type Points = BTreeMap<String, i64>;

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct DialogueNodeData {
    pub text: String,
    pub sprite: Option<String>,
    #[serde(default)]
    pub points: Points,
    /// What the player must already have earned before this node may be
    /// offered as a choice. Unmet nodes stay locked; see
    /// [`DialogueHistory::unlocked`](crate::history::DialogueHistory::unlocked).
    #[serde(default)]
    pub requires: Points,
    pub next: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct DialogueNode {
    label: String,
    data: DialogueNodeData,
}

impl DialogueNode {
    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn data(&self) -> &DialogueNodeData {
        &self.data
    }
}

impl BiHashItem for DialogueNode {
    type K1<'a> = &'a str;

    type K2<'a> = &'a DialogueNodeData;

    fn key1(&self) -> Self::K1<'_> {
        &self.label.as_str()
    }

    fn key2(&self) -> Self::K2<'_> {
        &self.data
    }

    bi_upcast!();
}

/// An NPC's dialogue *graph* — immutable content, exactly as authored.
///
/// Deliberately holds no cursor: where the player currently stands, and how
/// they got there, lives in [`DialogueHistory`](crate::history::DialogueHistory)
/// so that walking it back is a retraction rather than a bookkeeping problem.
#[derive(Serialize, Deserialize)]
pub struct NPCData {
    name: String,
    pronouns: String,
    dialogue: BiHashMap<DialogueNode>,
}

/// An NPC exactly as it is written in the asset JSON, where the dialogue is an
/// object keyed by node label rather than the flat list a [`BiHashMap`]
/// serializes to.
#[derive(Debug, Deserialize)]
struct RawNPCData {
    name: String,
    pronouns: String,
    dialogue: HashMap<String, DialogueNodeData>,
}

#[derive(Debug, Error)]
pub enum NPCParseError {
    #[error("malformed NPC JSON")]
    Malformed(#[from] serde_json::Error),
    #[error("nodes '{first}' and '{second}' have identical contents")]
    DuplicateNodeData { first: String, second: String },
    #[error("node '{from}' points at '{to}', which doesn't exist")]
    UnknownNextNode { from: String, to: String },
    #[error("label '{label}' doesn't exist")]
    UnknownLabel { label: String },
    #[error("'{to}' needs {need} {flag}, but only {have} has been earned")]
    Locked {
        to: String,
        flag: String,
        have: i64,
        need: i64,
    },
}

impl NPCData {
    /// Parses the bytes of an NPC asset (such as `assets/dialogue1.json`) into
    /// an [`NPCData`], taking bytes so it can be fed straight from
    /// `fetch_asset_bytes`.
    ///
    /// Because [`DialogueNodeData`] is the second key of the dialogue map, two
    /// nodes that say the same thing and go the same places cannot coexist;
    /// that collision is reported rather than silently dropping a node.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, NPCParseError> {
        let raw: RawNPCData = serde_json::from_slice(bytes)?;

        let labels: HashSet<&String> = raw.dialogue.keys().collect();
        for (label, data) in &raw.dialogue {
            for next in &data.next {
                if !labels.contains(next) {
                    return Err(NPCParseError::UnknownNextNode {
                        from: label.clone(),
                        to: next.clone(),
                    });
                }
            }
        }

        let mut dialogue = BiHashMap::with_capacity(raw.dialogue.len());
        for (label, data) in raw.dialogue {
            let node = DialogueNode { label, data };
            if let Err(duplicate) = dialogue.insert_unique(node) {
                let first = duplicate
                    .duplicates()
                    .first()
                    .map(|existing| existing.label.clone())
                    .unwrap_or_default();
                return Err(NPCParseError::DuplicateNodeData {
                    first,
                    second: duplicate.new_item().label.clone(),
                });
            }
        }

        Ok(NPCData {
            name: raw.name,
            pronouns: raw.pronouns,
            dialogue,
        })
    }

    /// The label every conversation with this NPC opens on.
    pub const START_LABEL: &'static str = "start";

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The node `label` names, if the graph has one.
    pub fn node(&self, label: &str) -> Option<&DialogueNode> {
        self.dialogue.get1(label)
    }

    /// Whether the player standing on `from` is allowed to pick `to`.
    pub fn transition_allowed(&self, from: &str, to: &str) -> Result<(), NPCParseError> {
        let node = self.node(from).ok_or(NPCParseError::UnknownLabel {
            label: from.to_string(),
        })?;
        if node.data.next.iter().any(|next| next == to) {
            Ok(())
        } else {
            Err(NPCParseError::UnknownNextNode {
                from: from.to_string(),
                to: to.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dialogue1() {
        let bytes = include_bytes!("../assets/test_dialogue.json");
        let npc = NPCData::from_json_slice(bytes).expect("dialogue1.json should parse");

        assert_eq!(npc.name, "Jane Doe");
        assert_eq!(npc.pronouns, "She/They");
        assert_eq!(npc.dialogue.len(), 2);

        let start = npc.dialogue.get1("start").expect("start node");
        assert_eq!(start.data.text, "Hello!");
        assert_eq!(start.data.next, vec!["end".to_string()]);

        let end = npc.dialogue.get1("end").expect("end node");
        assert!(end.data.next.is_empty());
    }

    #[test]
    fn rejects_dangling_next() {
        let json = br#"{
            "name": "Jane Doe",
            "pronouns": "She/They",
            "dialogue": { "start": { "text": "Hello!", "next": ["nowhere"] } }
        }"#;

        assert!(matches!(
            NPCData::from_json_slice(json),
            Err(NPCParseError::UnknownNextNode { .. })
        ));
    }
}
