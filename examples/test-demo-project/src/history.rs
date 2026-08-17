use std::path::Path;

use fold::pipeline::{Aggregate, FilterMap, FlatMap, Keyed, Map, Scored, terminal};
use fold::stream::Stream;
use serde::{Deserialize, Serialize};

use crate::dialogue::{NPCData, NPCParseError, Points};

/// One step along a path: `npc`'s conversation stood on `label` at depth
/// `step`, which put up `sprite` if it is one of the nodes that sets one.
///
/// `step` is the score, and it only ever increases along a live path, so
/// `(npc, step, label)` identifies exactly one insertion. Retracting it can't
/// collide with any other visit — including a revisit of the same node later
/// in the conversation, which lands at a different depth.
///
/// The sprite is copied off the node here rather than looked up downstream:
/// pipeline nodes are plain `fn`s with no access to the graph, so the only
/// way a branch can see a sprite is for the record to carry it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogueVisit {
    pub npc: String,
    pub step: u32,
    pub label: String,
    pub sprite: Option<String>,
    pub points: Points,
}

/// What a visit records beyond the `(npc, step)` its history entry is already
/// keyed and scored by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitPoint {
    pub label: String,
    pub sprite: Option<String>,
    pub points: Points,
}

/// `Keyed { key: npc, val: Scored { score: step, val: point } }` — the shape
/// [`terminal::KeyedRanked`] wants: one score-ordered run per NPC.
type HistoryEntry = Keyed<String, Scored<u32, VisitPoint>>;

/// The same shape as [`HistoryEntry`], but only visits that name a sprite
/// reach it, so a key's whole run is the portrait changes along the path.
type SpriteChange = Keyed<String, Scored<u32, String>>;

/// One affinity award: `how much` the visit adds to `(npc, flag)`.
///
/// Keyed by the pair rather than the NPC alone so `romance` and `trust` are
/// separate running totals that never have to be told about each other.
type PointAward = Keyed<(String, String), i64>;

// Plain `fn`s rather than closures: a closure's type can't be named, and the
// pipeline type appears in `Stream<D, P>`, so a closure here would make
// `DialogueHistory` impossible to write down as a struct field.
fn history_entry(visit: &DialogueVisit) -> HistoryEntry {
    Keyed::new(
        visit.npc.clone(),
        Scored::new(
            visit.step,
            VisitPoint {
                label: visit.label.clone(),
                sprite: visit.sprite.clone(),
                points: visit.points.clone(),
            },
        ),
    )
}

fn visited_label(visit: &DialogueVisit) -> String {
    visit.label.clone()
}

/// Drops the visits that leave the portrait alone, so the sprite branch holds
/// only the steps that actually changed it.
fn sprite_change(visit: &DialogueVisit) -> Option<SpriteChange> {
    let sprite = visit.sprite.clone()?;
    Some(Keyed::new(
        visit.npc.clone(),
        Scored::new(visit.step, sprite),
    ))
}

/// Fans one visit out into an award per flag its node grants — none for the
/// nodes that grant nothing, several for a node that moves two scores at once.
fn points_awarded(visit: &DialogueVisit) -> Vec<PointAward> {
    visit
        .points
        .iter()
        .map(|(flag, amount)| Keyed::new((visit.npc.clone(), flag.clone()), *amount))
        .collect()
}

/// The running total per `(npc, flag)`.
fn accumulate_points(total: &mut i64, amount: &i64, delta: isize) {
    *total += amount * delta as i64;
}

/// `Aggregate` folds the awards into a sum per key and republishes it as a
/// changelog, which [`terminal::Table`] keeps last-writer-wins — so the score
/// is a point read rather than a scan over the path.
type PointsSink = Aggregate<
    (String, String),
    i64,
    i64,
    fn(&mut i64, &i64, isize),
    terminal::Table<(String, String), i64>,
>;

type HistoryPipeline = (
    Map<
        fn(&DialogueVisit) -> HistoryEntry,
        terminal::KeyedRanked<String, u32, VisitPoint>,
        DialogueVisit,
        HistoryEntry,
    >,
    Map<fn(&DialogueVisit) -> String, terminal::Bag<String>, DialogueVisit, String>,
    FilterMap<
        fn(&DialogueVisit) -> Option<SpriteChange>,
        terminal::KeyedRanked<String, u32, String>,
        DialogueVisit,
        SpriteChange,
    >,
    FlatMap<fn(&DialogueVisit) -> Vec<PointAward>, PointsSink, DialogueVisit, PointAward>,
);

/// The sprite `label`'s node puts up, if it is one of the nodes that sets one.
fn sprite_of(npc: &NPCData, label: &str) -> Option<String> {
    npc.node(label)?.data().sprite.clone()
}

/// What `label`'s node awards, empty if it awards nothing.
///
/// Copied onto the visit for the same reason the sprite is: pipeline nodes are
/// plain `fn`s with no access to the graph, so an award the record doesn't
/// carry is an award no branch can see — and, more to the point, an award that
/// rewind couldn't take back.
fn points_of(npc: &NPCData, label: &str) -> Points {
    npc.node(label)
        .map(|node| node.data().points.clone())
        .unwrap_or_default()
}

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
                    // the portrait: the same per-NPC score-ordered runs, but
                    // only the steps that set a sprite
                    FilterMap::new(
                        sprite_change as fn(&DialogueVisit) -> Option<SpriteChange>,
                        terminal::KeyedRanked::new("sprite"),
                    ),
                    // affinity: every award on the live path, summed per
                    // (npc, flag). Gating reads this; nothing writes it back.
                    FlatMap::new(
                        points_awarded as fn(&DialogueVisit) -> Vec<PointAward>,
                        Aggregate::new(
                            "points",
                            accumulate_points as fn(&mut i64, &i64, isize),
                            terminal::Table::new("scores"),
                        ),
                    ),
                ),
            ),
        }
    }

    /// The record for where `npc`'s conversation stands, rebuilt byte-for-byte
    /// as it was inserted so it can be handed straight to a retraction.
    fn current_visit(&self, npc: &str) -> Option<DialogueVisit> {
        let key = npc.to_string();
        self.stream.rtx(|(history, _, _, _)| {
            history.max(&key).map(|top| DialogueVisit {
                npc: key.clone(),
                step: top.score,
                label: top.val.label,
                sprite: top.val.sprite,
                points: top.val.points,
            })
        })
    }

    /// Where `npc`'s conversation currently stands, as `(depth, label)`.
    ///
    /// The highest-scored visit *is* the cursor — there is no stored "current
    /// node" that could disagree with the history.
    pub fn current(&self, npc: &str) -> Option<(u32, String)> {
        self.current_visit(npc)
            .map(|visit| (visit.step, visit.label))
    }

    /// The sprite showing while the player stands where they are, if any node
    /// on the live path has set one.
    ///
    /// Nodes that leave `sprite` unset never reach this branch, so the
    /// highest-scored entry is the nearest node at or behind the cursor that
    /// did set one — exactly the "a portrait stays up until something replaces
    /// it" rule, without a rule anywhere.
    ///
    /// Rewind needs no help either: stepping back past the node that set the
    /// sprite retracts its entry here in the same transaction, and `max`
    /// reveals the runner-up — the portrait that was up before it.
    pub fn current_sprite(&self, npc: &str) -> Option<String> {
        let npc = npc.to_string();
        self.stream
            .rtx(|(_, _, sprite, _)| sprite.max(&npc).map(|top| top.val))
    }

    /// How much `flag` the player has earned with `npc` along the live path.
    ///
    /// A point read on the aggregate, not a walk of the history: the sum is
    /// already materialized, and it was maintained by the same deltas that
    /// maintain everything else.
    pub fn points(&self, npc: &str, flag: &str) -> i64 {
        let key = (npc.to_string(), flag.to_string());
        self.stream
            .rtx(|(_, _, _, scores)| scores.get(&key))
            .unwrap_or(0)
    }

    /// Every flag `npc` has a running score in, for the UI to show.
    pub fn scores(&self, npc: &str) -> Vec<(String, i64)> {
        let npc = npc.to_string();
        self.stream.rtx(|(_, _, _, scores)| {
            scores
                .iter()
                .filter(|((owner, _), _)| *owner == npc)
                .map(|((_, flag), total)| (flag, total))
                .collect::<Vec<_>>()
        })
    }

    /// Whether the player has earned everything `label` demands.
    ///
    /// Unknown labels count as unlocked; the graph check in
    /// [`advance`](Self::advance) is what rejects those.
    pub fn unlocked(&self, npc: &NPCData, label: &str) -> bool {
        self.unmet_requirement(npc, label).is_none()
    }

    /// The first requirement of `label` the player falls short of, as
    /// `(flag, have, need)`.
    fn unmet_requirement(&self, npc: &NPCData, label: &str) -> Option<(String, i64, i64)> {
        let node = npc.node(label)?;
        node.data().requires.iter().find_map(|(flag, need)| {
            let have = self.points(npc.name(), flag);
            (have < *need).then(|| (flag.clone(), have, *need))
        })
    }

    /// The choices on offer from where the player stands, each paired with
    /// whether it is unlocked yet.
    ///
    /// Locked entries are returned rather than dropped so the UI can decide
    /// between hiding them and showing them greyed out — a locked option the
    /// player can *see* is what makes a route feel earnable.
    pub fn options(&self, npc: &NPCData) -> Vec<(String, bool)> {
        let Some((_, label)) = self.current(npc.name()) else {
            return Vec::new();
        };
        let Some(node) = npc.node(&label) else {
            return Vec::new();
        };
        node.data()
            .next
            .iter()
            .map(|next| (next.clone(), self.unlocked(npc, next)))
            .collect()
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
            sprite: sprite_of(npc, NPCData::START_LABEL),
            points: points_of(npc, NPCData::START_LABEL),
        };
        self.stream.wtx(|tx| tx.insert(&root));
        (root.step, root.label)
    }

    /// Step forward to `to`, if the graph allows it from where the player is
    /// *and* the player has earned what `to` demands.
    ///
    /// The gate is checked here rather than only in the UI so a locked node
    /// can't be reached by a caller that didn't ask [`options`](Self::options)
    /// first.
    pub fn advance(&mut self, npc: &NPCData, to: &str) -> Result<(), NPCParseError> {
        let (step, from) = self.enter(npc);
        npc.transition_allowed(&from, to)?;
        if let Some((flag, have, need)) = self.unmet_requirement(npc, to) {
            return Err(NPCParseError::Locked {
                to: to.to_string(),
                flag,
                have,
                need,
            });
        }

        let visit = DialogueVisit {
            npc: npc.name().to_string(),
            step: step + 1,
            label: to.to_string(),
            sprite: sprite_of(npc, to),
            points: points_of(npc, to),
        };
        self.stream.wtx(|tx| tx.insert(&visit));
        Ok(())
    }

    /// Step back one node, returning whether there was anywhere to go.
    pub fn rewind(&mut self, npc: &str) -> bool {
        let Some(visit) = self.current_visit(npc) else {
            return false;
        };
        if visit.step == 0 {
            return false; // standing on the root; nothing behind it
        }

        self.stream.wtx(|tx| tx.remove(&visit));
        true
    }

    /// Walk the whole conversation back to its opening line.
    ///
    /// One transaction, so the path and everything derived from it either all
    /// roll back or none of it does.
    pub fn restart(&mut self, npc: &str) {
        let npc_key = npc.to_string();
        let visits: Vec<(DialogueVisit, i64)> = self.stream.rtx(|(history, _, _, _)| {
            history
                .iter(&npc_key)
                .filter(|(visit, _)| visit.score > 0)
                .map(|(visit, count)| {
                    (
                        DialogueVisit {
                            npc: npc_key.clone(),
                            step: visit.score,
                            label: visit.val.label,
                            sprite: visit.val.sprite,
                            points: visit.val.points,
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
        self.stream.rtx(|(history, _, _, _)| {
            history
                .iter(&npc)
                .map(|(visit, _)| visit.val.label)
                .collect::<Vec<_>>()
        })
    }

    /// How many times each node has been visited
    pub fn visited_counts(&self) -> Vec<(String, i64)> {
        self.stream
            .rtx(|(_, visited, _, _)| visited.iter().collect::<Vec<_>>())
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
    fn the_sprite_branch_holds_the_last_node_that_set_one() {
        let npc = NPCData::from_json_slice(include_bytes!("../assets/dialogue1.json"))
            .expect("should parse");
        let mut history = history("sprite");

        // `start` sets one, so it is up from the opening line
        history.enter(&npc);
        assert_eq!(
            history.current_sprite(npc.name()),
            Some("body2 34".to_string())
        );

        // `Say Hi` sets none: the portrait carries over untouched
        history.advance(&npc, "Say Hi").expect("legal");
        assert_eq!(
            history.current_sprite(npc.name()),
            Some("body2 34".to_string())
        );

        // ...and the other branch does set one
        history.rewind(npc.name());
        history.advance(&npc, "Screw You").expect("legal");
        assert_eq!(
            history.current_sprite(npc.name()),
            Some("body2 39".to_string())
        );

        // rewinding past it retracts that entry, and `max` falls back to the
        // sprite that was up before — nothing in `rewind` knows about sprites
        assert!(history.rewind(npc.name()));
        assert_eq!(
            history.current_sprite(npc.name()),
            Some("body2 34".to_string())
        );
    }

    #[test]
    fn restart_leaves_the_opening_sprite_up() {
        let npc = NPCData::from_json_slice(include_bytes!("../assets/dialogue1.json"))
            .expect("should parse");
        let mut history = history("sprite-restart");

        history.enter(&npc);
        history.advance(&npc, "Screw You").expect("legal");
        history.advance(&npc, "Screw You").expect("legal");

        history.restart(npc.name());
        assert_eq!(history.path(npc.name()), vec!["start".to_string()]);
        assert_eq!(
            history.current_sprite(npc.name()),
            Some("body2 34".to_string())
        );
    }

    /// Walks the romance route far enough to earn `n` points, from a fresh
    /// history.
    fn romance_route(name: &str, steps: &[&str]) -> (NPCData, DialogueHistory) {
        let npc = NPCData::from_json_slice(include_bytes!("../assets/dialogue1.json"))
            .expect("should parse");
        let mut history = history(name);
        history.enter(&npc);
        for step in steps {
            history.advance(&npc, step).expect("legal");
        }
        (npc, history)
    }

    #[test]
    fn awards_accumulate_along_the_path() {
        let (npc, history) = romance_route(
            "points",
            &["Say Hi", "You look happy today", "Ask her to walk with you"],
        );

        assert_eq!(history.points(npc.name(), "romance"), 3);
        assert_eq!(history.scores(npc.name()), vec![("romance".to_string(), 3)]);
    }

    #[test]
    fn a_gated_node_stays_locked_until_its_price_is_paid() {
        let (npc, mut history) = romance_route("locked", &["Say Hi"]);

        // one point in: the confession is on offer from `Alright`, but shut
        assert!(!history.unlocked(&npc, "I love you"));
        history.advance(&npc, "Alright").expect("legal");
        assert_eq!(
            history.options(&npc),
            vec![("I love you".to_string(), false), ("end".to_string(), true)]
        );

        // and the gate is not just a UI courtesy
        assert!(matches!(
            history.advance(&npc, "I love you"),
            Err(NPCParseError::Locked {
                flag,
                have: 1,
                need: 3,
                ..
            }) if flag == "romance"
        ));
        assert_eq!(
            history.current(npc.name()),
            Some((2, "Alright".to_string()))
        );
    }

    #[test]
    fn the_third_point_opens_the_gate() {
        let (npc, mut history) = romance_route(
            "unlock",
            &["Say Hi", "You look happy today", "Ask her to walk with you"],
        );

        assert!(history.unlocked(&npc, "I love you"));
        history.advance(&npc, "I love you").expect("earned it");
        assert_eq!(
            history.current(npc.name()),
            Some((4, "I love you".to_string()))
        );
    }

    #[test]
    fn rewinding_past_an_award_takes_the_point_back_and_re_locks_the_gate() {
        let (npc, mut history) = romance_route(
            "relock",
            &["Say Hi", "You look happy today", "Ask her to walk with you"],
        );
        assert!(history.unlocked(&npc, "I love you"));

        // stepping back off the third point retracts its award in the same
        // transaction — nothing in `rewind` knows affinity exists
        assert!(history.rewind(npc.name()));
        assert_eq!(history.points(npc.name(), "romance"), 2);
        assert!(!history.unlocked(&npc, "I love you"));

        history.restart(npc.name());
        assert_eq!(history.points(npc.name(), "romance"), 0);
        assert!(history.scores(npc.name()).is_empty());
    }

    #[test]
    fn a_rude_answer_costs_affection_every_time_it_is_repeated() {
        let (npc, mut history) = romance_route("rude", &["Screw You", "Screw You"]);
        assert_eq!(history.points(npc.name(), "romance"), -2);

        // revisits stack, so unwinding one gives exactly one point back
        history.rewind(npc.name());
        assert_eq!(history.points(npc.name(), "romance"), -1);
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
