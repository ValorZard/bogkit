//! The player's path through a conversation, as a fold stream.
//!
//! [`NPCData`] owns the dialogue *graph*; this owns the *cursor*. Each step
//! forward is a [`DialogueVisit`] pushed into fold with a positive delta, so
//! stepping back is the same record pushed with a negative one — fold's
//! native operation. Nothing here maintains an undo stack, because every view
//! hung off the pipeline retracts on its own: the position comes back from
//! `KeyedRanked::max` revealing the runner-up, and the per-node visit counts
//! in the `visited` bag decrement in the same transaction.
//!
//! That is the whole reason to reach for fold instead of a `Vec<String>`: add
//! a branch to the pipeline (a flag table, a quest counter, an index of what
//! the player has been told) and rewind keeps working with no new code.

use std::path::Path;

use fold::pipeline::{Keyed, Map, Scored, terminal};
use fold::stream::Stream;
use serde::{Deserialize, Serialize};

use crate::dialogue::{NPCData, NPCParseError};

/// One step along a path: `npc`'s conversation stood on `label` at depth
/// `step`.
///
/// `step` is the score, and it only ever increases along a live path, so
/// `(npc, step, label)` identifies exactly one insertion. Retracting it can't
/// collide with any other visit — including a revisit of the same node later
/// in the conversation, which lands at a different depth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogueVisit {
    pub npc: String,
    pub step: u32,
    pub label: String,
}

/// `Keyed { key: npc, val: Scored { score: step, val: label } }` — the shape
/// [`terminal::KeyedRanked`] wants: one score-ordered run per NPC.
type HistoryEntry = Keyed<String, Scored<u32, String>>;

// Plain `fn`s rather than closures: a closure's type can't be named, and the
// pipeline type appears in `Stream<D, P>`, so a closure here would make
// `DialogueHistory` impossible to write down as a struct field.
fn history_entry(visit: &DialogueVisit) -> HistoryEntry {
    Keyed::new(
        visit.npc.clone(),
        Scored::new(visit.step, visit.label.clone()),
    )
}

fn visited_label(visit: &DialogueVisit) -> String {
    visit.label.clone()
}

type HistoryPipeline = (
    Map<
        fn(&DialogueVisit) -> HistoryEntry,
        terminal::KeyedRanked<String, u32, String>,
        DialogueVisit,
        HistoryEntry,
    >,
    Map<fn(&DialogueVisit) -> String, terminal::Bag<String>, DialogueVisit, String>,
);

/// Every conversation's path, materialized incrementally and persisted.
pub struct DialogueHistory {
    stream: Stream<DialogueVisit, HistoryPipeline>,
}

impl DialogueHistory {
    /// Open (or resume) the history store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Self {
        DialogueHistory {
            stream: Stream::new(
                path,
                (
                    // the path itself: per-NPC, ordered by depth
                    Map::new(
                        history_entry as fn(&DialogueVisit) -> HistoryEntry,
                        terminal::KeyedRanked::new("history"),
                    ),
                    // a derived view, to show retraction reaching past the
                    // cursor: how many times each node sits on a live path
                    Map::new(
                        visited_label as fn(&DialogueVisit) -> String,
                        terminal::Bag::new("visited"),
                    ),
                ),
            ),
        }
    }

    /// Where `npc`'s conversation currently stands, as `(depth, label)`.
    ///
    /// The highest-scored visit *is* the cursor — there is no stored "current
    /// node" that could disagree with the history.
    pub fn current(&self, npc: &str) -> Option<(u32, String)> {
        let npc = npc.to_string();
        self.stream
            .rtx(|(history, _)| history.max(&npc).map(|top| (top.score, top.val)))
    }

    /// Open the conversation if it has never been had, then report where it
    /// stands.
    pub fn enter(&mut self, npc: &NPCData) -> (u32, String) {
        if let Some(current) = self.current(npc.name()) {
            return current;
        }
        let root = DialogueVisit {
            npc: npc.name().to_string(),
            step: 0,
            label: NPCData::START_LABEL.to_string(),
        };
        self.stream.wtx(|tx| tx.insert(&root));
        (root.step, root.label)
    }

    /// Step forward to `to`, if the graph allows it from where the player is.
    pub fn advance(&mut self, npc: &NPCData, to: &str) -> Result<(), NPCParseError> {
        let (step, from) = self.enter(npc);
        npc.transition_allowed(&from, to)?;

        self.stream.wtx(|tx| {
            tx.insert(&DialogueVisit {
                npc: npc.name().to_string(),
                step: step + 1,
                label: to.to_string(),
            })
        });
        Ok(())
    }

    /// Step back one node, returning whether there was anywhere to go.
    pub fn rewind(&mut self, npc: &str) -> bool {
        let Some((step, label)) = self.current(npc) else {
            return false;
        };
        if step == 0 {
            return false; // standing on the root; nothing behind it
        }

        let visit = DialogueVisit {
            npc: npc.to_string(),
            step,
            label,
        };
        self.stream.wtx(|tx| tx.remove(&visit));
        true
    }

    /// Walk the whole conversation back to its opening line.
    ///
    /// One transaction, so the path and everything derived from it either all
    /// roll back or none of it does.
    pub fn restart(&mut self, npc: &str) {
        let npc_key = npc.to_string();
        let visits: Vec<(DialogueVisit, i64)> = self.stream.rtx(|(history, _)| {
            history
                .iter(&npc_key)
                .filter(|(visit, _)| visit.score > 0)
                .map(|(visit, count)| {
                    (
                        DialogueVisit {
                            npc: npc_key.clone(),
                            step: visit.score,
                            label: visit.val,
                        },
                        count,
                    )
                })
                .collect()
        });
        if visits.is_empty() {
            return;
        }

        self.stream.wtx(|tx| {
            for (visit, count) in &visits {
                tx.push(visit, -*count as isize);
            }
        });
    }

    /// The current path from the opening line to the cursor, in order.
    pub fn path(&self, npc: &str) -> Vec<String> {
        let npc = npc.to_string();
        self.stream.rtx(|(history, _)| {
            history
                .iter(&npc)
                .map(|(visit, _)| visit.val)
                .collect::<Vec<_>>()
        })
    }

    /// How many times each node has been visited
    pub fn visited_counts(&self) -> Vec<(String, i64)> {
        self.stream
            .rtx(|(_, visited)| visited.iter().collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npc() -> NPCData {
        NPCData::from_json_slice(include_bytes!("../assets/test_dialogue.json"))
            .expect("should parse")
    }

    fn history(name: &str) -> DialogueHistory {
        let path = std::env::temp_dir().join(format!("bogkit-history-test-{name}.db"));
        let _ = std::fs::remove_dir_all(&path);
        DialogueHistory::open(path)
    }

    #[test]
    fn rewind_restores_the_previous_node_and_its_derived_views() {
        let npc = npc();
        let mut history = history("rewind");

        assert_eq!(history.enter(&npc), (0, "start".to_string()));
        history.advance(&npc, "end").expect("start -> end is legal");

        assert_eq!(history.current(npc.name()), Some((1, "end".to_string())));
        assert!(history.visited_counts().contains(&("end".to_string(), 1)));

        assert!(history.rewind(npc.name()));
        assert_eq!(history.current(npc.name()), Some((0, "start".to_string())));
        // the derived view retracted too, without being told to
        assert!(!history.visited_counts().iter().any(|(l, _)| l == "end"));

        // nothing behind the opening line
        assert!(!history.rewind(npc.name()));
    }

    #[test]
    fn rejects_a_step_the_graph_does_not_allow() {
        let npc = npc();
        let mut history = history("illegal");
        history.enter(&npc);

        assert!(matches!(
            history.advance(&npc, "nowhere"),
            Err(NPCParseError::UnknownNextNode { .. })
        ));
        assert_eq!(history.current(npc.name()), Some((0, "start".to_string())));
    }

    #[test]
    fn rewinding_then_branching_leaves_no_trace_of_the_abandoned_path() {
        let npc = NPCData::from_json_slice(include_bytes!("../assets/dialogue1.json"))
            .expect("should parse");
        let mut history = history("branch");

        history.enter(&npc);
        history.advance(&npc, "Say Hi").expect("legal");
        history.advance(&npc, "Alright").expect("legal");
        assert_eq!(history.path(npc.name()), vec!["start", "Say Hi", "Alright"]);

        history.rewind(npc.name());
        history.rewind(npc.name());
        history
            .advance(&npc, "Screw You")
            .expect("Should be able to select this");

        assert_eq!(history.path(npc.name()), vec!["start", "Screw You"]);
        // none of the options from the other branch should be in the history
        assert!(
            !history
                .visited_counts()
                .iter()
                .any(|(label, _)| label == "Say Hi" || label == "Alright")
        );
    }

    #[test]
    fn restart_walks_the_whole_path_back() {
        let npc = npc();
        let mut history = history("restart");
        history.enter(&npc);
        history.advance(&npc, "end").expect("start -> end is legal");

        history.restart(npc.name());
        assert_eq!(history.path(npc.name()), vec!["start".to_string()]);
    }
}
