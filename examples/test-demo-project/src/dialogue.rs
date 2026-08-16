use std::collections::{HashMap, HashSet};

use iddqd::{BiHashItem, BiHashMap, bi_upcast};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct DialogueNodeData {
    pub text: String,
    pub next: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct DialogueNode {
    label: String,
    data: DialogueNodeData,
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

#[derive(Serialize, Deserialize)]
pub struct NPCData {
    name: String,
    pronouns: String,
    dialogue: BiHashMap<DialogueNode>,
    current_dialogue_node: String,
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
            current_dialogue_node: "start".to_string(),
        })
    }

    pub fn get_current_dialog(&self) -> Option<(String, &DialogueNode)> {
        let dialogue_node = self.dialogue.get1(self.current_dialogue_node.as_str())?;
        Some((self.current_dialogue_node.clone(), dialogue_node))
    }

    pub fn set_next_dialog(&mut self, next: String) -> Result<(), NPCParseError> {
        let dialogue_node = self
            .dialogue
            .get1(self.current_dialogue_node.as_str())
            .ok_or(NPCParseError::UnknownLabel {
                label: self.current_dialogue_node.clone(),
            })?;
        if !dialogue_node.key2().next.contains(&next) {
            return Err(NPCParseError::UnknownNextNode {
                from: self.current_dialogue_node.clone(),
                to: next,
            });
        } else {
            self.current_dialogue_node = next;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dialogue1() {
        let bytes = include_bytes!("../assets/dialogue1.json");
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
